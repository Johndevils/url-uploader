use std::{sync::Arc, time::Duration};

use anyhow::Result; use async_read_progress::TokioAsyncReadProgressExt; use dashmap::{DashMap, DashSet}; use futures::TryStreamExt; use grammers_client::{ button, reply_markup, types::{CallbackQuery, Chat, Message, User}, Client, InputMessage, Update, }; use log::{error, info, warn}; use reqwest::Url; use scopeguard::defer; use stream_cancel::{Trigger, Valved}; use tokio::sync::Mutex; use tokio_util::compat::FuturesAsyncReadCompatExt;

use crate::command::{parse_command, Command};

#[derive(Debug)] pub struct Bot { client: Client, me: User, http: reqwest::Client, locks: Arc<DashSet<i64>>, started_by: Arc<DashMap<i64, i64>>, triggers: Arc<DashMap<i64, Trigger>>, }

impl Bot { pub async fn new(client: Client) -> Result<Arc<Self>> { let me = client.get_me().await?; Ok(Arc::new(Self { client, me, http: reqwest::Client::builder() .connect_timeout(Duration::from_secs(10)) .user_agent("Mozilla/5.0") .build()?, locks: Arc::new(DashSet::new()), started_by: Arc::new(DashMap::new()), triggers: Arc::new(DashMap::new()), })) }

pub async fn run(self: Arc<Self>) {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received Ctrl+C, exiting");
                break;
            },
            Ok(update) = self.client.next_update() => {
                let this = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = this.handle_update(update).await {
                        error!("Update error: {}", e);
                    }
                });
            }
        }
    }
}

async fn handle_update(&self, update: Update) -> Result<()> {
    match update {
        Update::NewMessage(msg) => self.handle_message(msg).await,
        Update::CallbackQuery(query) => self.handle_callback(query).await,
        _ => Ok(()),
    }
}

async fn handle_message(&self, msg: Message) -> Result<()> {
    match msg.chat() {
        Chat::User(_) | Chat::Group(_) => {}
        _ => return Ok(()),
    }

    let command = parse_command(msg.text());
    if let Some(cmd) = command {
        if let Some(via) = &cmd.via {
            if via.to_lowercase() != self.me.username().unwrap_or_default().to_lowercase() {
                return Ok(());
            }
        }
        if let Chat::Group(_) = msg.chat() {
            if cmd.name == "start" && cmd.via.is_none() {
                return Ok(());
            }
        }
        match cmd.name.as_str() {
            "start" => return self.handle_start(msg).await,
            "upload" => return self.handle_upload(msg, cmd).await,
            _ => {}
        }
    }

    if let Chat::User(_) = msg.chat() {
        if let Ok(url) = Url::parse(msg.text()) {
            return self.handle_url(msg, url).await;
        }
    }

    Ok(())
}

async fn handle_start(&self, msg: Message) -> Result<()> {
    msg.reply(InputMessage::html(
        "📁 <b>Hi! Need a file uploaded? Just send the link!</b>\nIn groups, use <code>/upload &lt;url&gt;</code>\n\n🌟 <b>Features:</b>\n• Free & fast\n• <a href=\"https://github.com/altfoxie/url-uploader\">Open source</a>\n• Uploads files up to 2GB\n• Redirect-friendly",
    )).await?;
    Ok(())
}

async fn handle_upload(&self, msg: Message, cmd: Command) -> Result<()> {
    let url = match cmd.arg {
        Some(url) => url,
        None => {
            msg.reply("Please specify a URL").await?;
            return Ok(());
        }
    };

    let url = match Url::parse(&url) {
        Ok(url) => url,
        Err(e) => {
            msg.reply(format!("Invalid URL: {}", e)).await?;
            return Ok(());
        }
    };

    self.handle_url(msg, url).await
}

async fn handle_url(&self, msg: Message, url: Url) -> Result<()> {
    let sender = msg.sender().ok_or_else(|| anyhow::anyhow!("No sender"))?;

    if !self.locks.insert(msg.chat().id()) {
        msg.reply("✋ Whoa, slow down! There's already an active upload in this chat.").await?;
        return Ok(());
    }
    self.started_by.insert(msg.chat().id(), sender.id());

    defer! {
        self.locks.remove(&msg.chat().id());
        self.started_by.remove(&msg.chat().id());
    }

    let response = self.http.get(url.clone()).send().await?;
    let length = response.content_length().unwrap_or_default() as usize;
    let name = url
        .path_segments()
        .and_then(|segments| segments.last())
        .unwrap_or("file")
        .to_string();

    if length == 0 {
        msg.reply("⚠️ File is empty").await?;
        return Ok(());
    }

    if length > 2 * 1024 * 1024 * 1024 {
        msg.reply("⚠️ File is too large").await?;
        return Ok(());
    }

    let (trigger, stream) = Valved::new(
        response
            .bytes_stream()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
    );
    self.triggers.insert(msg.chat().id(), trigger);

    defer! {
        self.triggers.remove(&msg.chat().id());
    }

    let reply_markup = Arc::new(reply_markup::inline(vec![vec![button::inline("⛔ Cancel", "cancel")]]));
    let status = Arc::new(Mutex::new(
        msg.reply(InputMessage::html(format!("🚀 Starting upload of <code>{}</code>...", name))
            .reply_markup(reply_markup.as_ref())).await?
    ));

    let mut stream = stream.into_async_read().compat().report_progress(Duration::from_secs(3), |progress| {
        let status = status.clone();
        let name = name.clone();
        let reply_markup = reply_markup.clone();
        tokio::spawn(async move {
            status.lock().await.edit(
                InputMessage::html(format!("⏳ Uploading <code>{}</code> <b>({:.2}%)</b>\n<i>{} / {}</i>",
                    name,
                    progress as f64 / length as f64 * 100.0,
                    bytesize::to_string(progress as u64, true),
                    bytesize::to_string(length as u64, true)))
                .reply_markup(reply_markup.as_ref())
            ).await.ok();
        });
    });

    let start_time = chrono::Utc::now();
    let file = self.client.upload_stream(&mut stream, length, name.clone()).await?;
    let elapsed = chrono::Utc::now() - start_time;

    if name.to_lowercase().ends_with(".mp4") {
        msg.reply(InputMessage::video(file)).await?;
    } else {
        msg.reply(InputMessage::document(file)).await?;
    }

    status.lock().await.delete().await?;

    Ok(())
}

async fn handle_callback(&self, query: CallbackQuery) -> Result<()> {
    match query.data() {
        b"cancel" => self.handle_cancel(query).await,
        _ => Ok(())
    }
}

async fn handle_cancel(&self, query: CallbackQuery) -> Result<()> {
    let user_id = self.started_by.get(&query.chat().id()).map(|v| *v);
    if user_id != Some(query.sender().id()) {
        query.answer().alert("⚠️ You can't cancel another user's upload").send().await?;
        return Ok(());
    }
    if let Some((_chat_id, trigger)) = self.triggers.remove(&query.chat().id()) {
        drop(trigger);
        query.load_message().await?.edit("⛔ Upload cancelled").await?;
        query.answer().send().await?;
    }
    Ok(())
}

}

