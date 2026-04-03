use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde_json::{json, Value};

use crate::error::NoteDeckError;
use crate::models::{
    AuthResult, ChatMessage, CreateNoteParams, NormalizedDriveFile, NormalizedNote,
    NormalizedNoteReaction, NormalizedNotification, NormalizedUser, NormalizedUserDetail,
    RawCreateNoteResponse, RawDriveFile, RawEmojisResponse, RawMiAuthResponse, RawNote,
    RawNoteReaction, RawNotification, Antenna, Channel, Clip, RawUser, RawUserDetail,
    SearchOptions, ServerEmoji, TimelineOptions, TimelineType, UserList,
};


/// Maximum response body size (50 MB) to prevent memory exhaustion from malicious servers.
const MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;

/// Options for `search_users`.
#[derive(Default)]
pub struct SearchUsersOptions<'a> {
    pub query: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub state: Option<&'a str>,
    pub limit: i64,
    pub offset: Option<i64>,
}

/// Apply sinceId/untilId pagination to a JSON params object.
fn apply_pagination(params: &mut Value, since_id: Option<&str>, until_id: Option<&str>) {
    if let Some(id) = since_id {
        params["sinceId"] = json!(id);
    }
    if let Some(id) = until_id {
        params["untilId"] = json!(id);
    }
}

pub struct MisskeyClient {
    client: Client,
    /// Override base URL for testing (e.g. "http://127.0.0.1:PORT").
    /// When set, requests use `{base_url}/api/{endpoint}` instead of `https://{host}/api/{endpoint}`.
    #[cfg(test)]
    base_url: Option<String>,
}

impl MisskeyClient {
    pub fn new() -> Result<Self, NoteDeckError> {
        Ok(Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .pool_max_idle_per_host(4)
                .build()?,
            #[cfg(test)]
            base_url: None,
        })
    }

    #[cfg(test)]
    fn with_base_url(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: Some(base_url.to_string()),
        }
    }

    fn api_url(&self, host: &str, endpoint: &str) -> String {
        #[cfg(test)]
        if let Some(ref base) = self.base_url {
            return format!("{base}/api/{endpoint}");
        }
        format!("https://{host}/api/{endpoint}")
    }

    /// Read the response body with a streaming size limit to prevent DoS.
    /// Enforces the limit incrementally so chunked responses cannot bypass it.
    async fn read_body_limited(
        res: reqwest::Response,
        endpoint: &str,
    ) -> Result<String, NoteDeckError> {
        let content_len = res.content_length();
        if let Some(len) = content_len {
            if len > MAX_RESPONSE_BYTES as u64 {
                return Err(NoteDeckError::Api {
                    endpoint: endpoint.to_string(),
                    status: 0,
                    message: "Response too large".to_string(),
                });
            }
        }
        let mut buf = Vec::with_capacity(content_len.unwrap_or(4096).min(MAX_RESPONSE_BYTES as u64) as usize);
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(NoteDeckError::from)?;
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_RESPONSE_BYTES {
                return Err(NoteDeckError::Api {
                    endpoint: endpoint.to_string(),
                    status: 0,
                    message: "Response too large".to_string(),
                });
            }
        }
        String::from_utf8(buf).map_err(|_| NoteDeckError::Api {
            endpoint: endpoint.to_string(),
            status: 0,
            message: "Invalid UTF-8 in response".to_string(),
        })
    }

    pub async fn request(
        &self,
        host: &str,
        token: &str,
        endpoint: &str,
        mut params: Value,
    ) -> Result<Value, NoteDeckError> {
        if let Some(obj) = params.as_object_mut() {
            if !token.is_empty() {
                obj.insert("i".to_string(), json!(token));
            }
        }

        let res = self
            .client
            .post(self.api_url(host, endpoint))
            .json(&params)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status().as_u16();
            let detail = match res.json::<Value>().await {
                Ok(body) => body
                    .pointer("/error/message")
                    .or_else(|| body.pointer("/error/code"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
                Err(_) => None,
            };
            let message = match detail {
                Some(d) => format!("{endpoint}: {d}"),
                None => format!("{endpoint} ({status})"),
            };
            return Err(NoteDeckError::Api {
                endpoint: endpoint.to_string(),
                status,
                message,
            });
        }

        let text = Self::read_body_limited(res, endpoint).await?;
        if text.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(&text).map_err(NoteDeckError::from)
        }
    }

    pub async fn get_timeline(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        timeline_type: TimelineType,
        options: TimelineOptions,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let endpoint = timeline_type.api_endpoint();
        let mut params = json!({ "limit": options.limit() });
        apply_pagination(&mut params, options.since_id.as_deref(), options.until_id.as_deref());
        if let Some(ref f) = options.filters {
            if let Some(v) = f.with_renotes {
                params["withRenotes"] = json!(v);
            }
            if let Some(v) = f.with_replies {
                params["withReplies"] = json!(v);
            }
            if let Some(v) = f.with_files {
                params["withFiles"] = json!(v);
            }
            if let Some(v) = f.with_bots {
                params["withBots"] = json!(v);
                // Some forks use excludeBots (inverse semantics)
                params["excludeBots"] = json!(!v);
            }
            if let Some(v) = f.with_sensitive {
                params["withSensitive"] = json!(v);
                params["excludeNsfw"] = json!(!v);
            }
        }
        if let Some(ref id) = options.list_id {
            params["listId"] = json!(id);
        }

        let data = self.request(host, token, &endpoint, params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_user_lists(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<UserList>, NoteDeckError> {
        let data = self.request(host, token, "users/lists/list", json!({})).await?;
        let lists: Vec<UserList> = serde_json::from_value(data)?;
        Ok(lists)
    }

    pub async fn get_antennas(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<Antenna>, NoteDeckError> {
        let data = self.request(host, token, "antennas/list", json!({})).await?;
        let antennas: Vec<Antenna> = serde_json::from_value(data)?;
        Ok(antennas)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_antenna_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        antenna_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({
            "antennaId": antenna_id,
            "limit": limit,
        });
        apply_pagination(&mut params, since_id, until_id);
        let data = self.request(host, token, "antennas/notes", params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_favorites(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        apply_pagination(&mut params, since_id, until_id);
        let data = self.request(host, token, "i/favorites", params).await?;
        // i/favorites returns [{ id, note: {...}, ... }]
        let items: Vec<Value> = serde_json::from_value(data)?;
        let mut notes = Vec::with_capacity(items.len());
        for mut item in items {
            if let Some(note_val) = item.get_mut("note").map(Value::take) {
                let raw: RawNote = serde_json::from_value(note_val)?;
                notes.push(raw.normalize(account_id, host));
            }
        }
        Ok(notes)
    }

    pub async fn get_featured_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        limit: i64,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let params = json!({ "limit": limit });
        let data = self
            .request(host, token, "notes/featured", params)
            .await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_clips(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<Clip>, NoteDeckError> {
        let data = self.request(host, token, "clips/list", json!({})).await?;
        let clips: Vec<Clip> = serde_json::from_value(data)?;
        Ok(clips)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_clip_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        clip_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({
            "clipId": clip_id,
            "limit": limit,
        });
        apply_pagination(&mut params, since_id, until_id);
        let data = self.request(host, token, "clips/notes", params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_channels(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<Channel>, NoteDeckError> {
        let search_fut = self.request(host, token, "channels/search", json!({"query": "", "limit": 100}));
        let featured_fut = self.request(host, token, "channels/featured", json!({}));

        let (followed, favorites, owned, featured, search) = if token.is_empty() {
            let (fe, s) = tokio::join!(featured_fut, search_fut);
            (None, None, None, Some(fe), Some(s))
        } else {
            let (fo, fa, o, fe, s) = tokio::join!(
                self.request(host, token, "channels/followed", json!({"limit": 100})),
                self.request(host, token, "channels/my-favorites", json!({"limit": 100})),
                self.request(host, token, "channels/owned", json!({"limit": 100})),
                featured_fut,
                search_fut,
            );
            (Some(fo), Some(fa), Some(o), Some(fe), Some(s))
        };

        let mut seen = std::collections::HashSet::with_capacity(128);
        let mut channels = Vec::with_capacity(128);

        // User's own channels first, then public channels
        for data in [followed, favorites, owned, featured, search].into_iter().flatten().flatten() {
            if let Ok(list) = serde_json::from_value::<Vec<Channel>>(data) {
                for ch in list {
                    if seen.insert(ch.id.clone()) {
                        channels.push(ch);
                    }
                }
            }
        }

        Ok(channels)
    }

    pub async fn search_channels(
        &self,
        host: &str,
        token: &str,
        query: &str,
    ) -> Result<Vec<Channel>, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "channels/search",
                json!({"query": query, "limit": 100}),
            )
            .await?;
        let channels: Vec<Channel> = serde_json::from_value(data)?;
        Ok(channels)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_channel_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        channel_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({
            "channelId": channel_id,
            "limit": limit,
        });
        apply_pagination(&mut params, since_id, until_id);
        let data = self
            .request(host, token, "channels/timeline", params)
            .await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_mentions(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        apply_pagination(&mut params, since_id, until_id);
        if let Some(v) = visibility {
            params["visibility"] = json!(v);
        }
        let data = self.request(host, token, "notes/mentions", params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_note(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        note_id: &str,
    ) -> Result<NormalizedNote, NoteDeckError> {
        let data = self
            .request(host, token, "notes/show", json!({ "noteId": note_id }))
            .await?;
        let raw: RawNote = serde_json::from_value(data)?;
        Ok(raw.normalize(account_id, host))
    }

    pub async fn create_note(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        params: CreateNoteParams,
    ) -> Result<NormalizedNote, NoteDeckError> {
        let mut body = json!({});
        if let Some(ref text) = params.text {
            body["text"] = json!(text);
        }
        if let Some(ref cw) = params.cw {
            body["cw"] = json!(cw);
        }
        if let Some(ref vis) = params.visibility {
            body["visibility"] = json!(vis);
        }
        if let Some(local_only) = params.local_only {
            body["localOnly"] = json!(local_only);
        }
        if let Some(ref flags) = params.mode_flags {
            for (key, value) in flags {
                // Only allow isNoteIn*Mode flags (e.g., isNoteInYamiMode)
                if key.starts_with("isNoteIn") && key.ends_with("Mode") && key.len() <= 30 {
                    body[key] = json!(value);
                }
            }
        }
        if let Some(ref id) = params.reply_id {
            body["replyId"] = json!(id);
        }
        if let Some(ref id) = params.renote_id {
            body["renoteId"] = json!(id);
        }
        if let Some(ref ids) = params.file_ids {
            body["fileIds"] = json!(ids);
        }
        if let Some(ref poll) = params.poll {
            body["poll"] = json!(poll);
        }
        if let Some(ref scheduled_at) = params.scheduled_at {
            body["scheduledAt"] = json!(scheduled_at);
        }

        let data = self.request(host, token, "notes/create", body).await?;
        let raw: RawCreateNoteResponse =
            serde_json::from_value(data)?;
        Ok(raw.created_note.normalize(account_id, host))
    }

    pub async fn create_reaction(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
        reaction: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "notes/reactions/create",
            json!({ "noteId": note_id, "reaction": reaction }),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_reaction(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "notes/reactions/delete",
            json!({ "noteId": note_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn get_note_reactions(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
        reaction_type: Option<&str>,
        limit: u32,
        until_id: Option<&str>,
    ) -> Result<Vec<NormalizedNoteReaction>, NoteDeckError> {
        let mut params = json!({ "noteId": note_id, "limit": limit });
        if let Some(rt) = reaction_type {
            params["type"] = json!(rt);
        }
        if let Some(uid) = until_id {
            params["untilId"] = json!(uid);
        }
        let data = self.request(host, token, "notes/reactions", params).await?;
        let raw: Vec<RawNoteReaction> = serde_json::from_value(data)?;
        Ok(raw.into_iter().map(Into::into).collect())
    }

    pub async fn update_note(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
        params: CreateNoteParams,
    ) -> Result<(), NoteDeckError> {
        let mut body = json!({ "noteId": note_id });
        if let Some(ref text) = params.text {
            body["text"] = json!(text);
        }
        if let Some(ref cw) = params.cw {
            body["cw"] = json!(cw);
        }
        if let Some(ref ids) = params.file_ids {
            body["fileIds"] = json!(ids);
        }
        self.request(host, token, "notes/update", body).await?;
        Ok(())
    }

    pub async fn upload_file(
        &self,
        host: &str,
        token: &str,
        file_name: &str,
        file_data: Vec<u8>,
        content_type: &str,
        is_sensitive: bool,
    ) -> Result<NormalizedDriveFile, NoteDeckError> {
        let file_part = Part::bytes(file_data)
            .file_name(file_name.to_string())
            .mime_str(content_type)
            .map_err(|e| NoteDeckError::Api {
                endpoint: "drive/files/create".to_string(),
                status: 0,
                message: e.to_string(),
            })?;

        let form = Form::new()
            .text("i", token.to_string())
            .text("isSensitive", is_sensitive.to_string())
            .part("file", file_part);

        let url = self.api_url(host, "drive/files/create");
        let resp = self.client.post(&url).multipart(form).send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let message = Self::read_body_limited(resp, "drive/files/create")
                .await
                .unwrap_or_default();
            return Err(NoteDeckError::Api {
                endpoint: "drive/files/create".to_string(),
                status,
                message,
            });
        }

        let text = Self::read_body_limited(resp, "drive/files/create").await?;
        let raw: RawDriveFile = serde_json::from_str(&text)?;
        Ok(NormalizedDriveFile::from(raw))
    }

    pub async fn create_favorite(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "notes/favorites/create",
            json!({ "noteId": note_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_favorite(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "notes/favorites/delete",
            json!({ "noteId": note_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_note(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "notes/delete",
            json!({ "noteId": note_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn get_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<NormalizedUser, NoteDeckError> {
        let data = self
            .request(host, token, "users/show", json!({ "userId": user_id }))
            .await?;
        let raw: RawUser = serde_json::from_value(data)?;
        Ok(raw.into())
    }

    pub async fn get_user_detail(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<NormalizedUserDetail, NoteDeckError> {
        let data = self
            .request(host, token, "users/show", json!({ "userId": user_id }))
            .await?;
        let raw: RawUserDetail = serde_json::from_value(data)?;
        Ok(raw.normalize())
    }

    pub async fn get_server_emojis(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<ServerEmoji>, NoteDeckError> {
        let data = self.request(host, token, "emojis", json!({})).await?;
        let raw: RawEmojisResponse = serde_json::from_value(data)?;
        Ok(raw.emojis.into_iter().map(ServerEmoji::from).collect())
    }

    pub async fn get_pinned_reactions(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Vec<String>, NoteDeckError> {
        // New Misskey preferences: scope ['client', 'preferences', 'sync'], key 'default:emojiPalettes'
        // Format: [[scope, palettes], ...] where palettes is [{id, name, emojis}, ...]
        if let Ok(data) = self.request(
            host, token, "i/registry/get",
            json!({ "scope": ["client", "preferences", "sync"], "key": "default:emojiPalettes" }),
        ).await {
            if let Some(emojis) = Self::extract_reaction_palette(&data) {
                if !emojis.is_empty() {
                    return Ok(emojis);
                }
            }
        }

        Ok(vec![])
    }

    fn extract_reaction_palette(data: &Value) -> Option<Vec<String>> {
        let entries = data.as_array()?;
        let (_, palettes_val) = entries.first().and_then(|e| {
            let arr = e.as_array()?;
            Some((arr.first()?, arr.get(1)?))
        })?;
        let palettes = palettes_val.as_array()?;
        let first = palettes.first()?;
        let emojis = first.get("emojis")?.as_array()?;
        Some(emojis.iter().filter_map(|v| v.as_str().map(String::from)).collect())
    }

    pub async fn get_user_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        user_id: &str,
        options: TimelineOptions,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({ "userId": user_id, "limit": options.limit() });
        apply_pagination(&mut params, options.since_id.as_deref(), options.until_id.as_deref());
        let data = self.request(host, token, "users/notes", params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn search_notes(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        query: &str,
        options: SearchOptions,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let mut params = json!({ "query": query, "limit": options.limit() });
        apply_pagination(&mut params, options.since_id.as_deref(), options.until_id.as_deref());
        if let Some(d) = options.since_date {
            params["sinceDate"] = json!(d);
        }
        if let Some(d) = options.until_date {
            params["untilDate"] = json!(d);
        }
        let data = self.request(host, token, "notes/search", params).await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_notifications(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        options: TimelineOptions,
    ) -> Result<Vec<NormalizedNotification>, NoteDeckError> {
        let mut params = json!({ "limit": options.limit() });
        apply_pagination(&mut params, options.since_id.as_deref(), options.until_id.as_deref());
        let data = self
            .request(host, token, "i/notifications", params)
            .await?;
        let raw: Vec<RawNotification> =
            serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_notifications_grouped(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        options: TimelineOptions,
    ) -> Result<Vec<NormalizedNotification>, NoteDeckError> {
        let mut params = json!({ "limit": options.limit() });
        apply_pagination(&mut params, options.since_id.as_deref(), options.until_id.as_deref());
        let data = self
            .request(host, token, "i/notifications-grouped", params)
            .await?;
        let raw: Vec<RawNotification> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    // --- Auth ---

    pub async fn complete_auth(
        &self,
        host: &str,
        session_id: &str,
    ) -> Result<AuthResult, NoteDeckError> {
        let res = self
            .client
            .post(self.api_url(host, &format!("miauth/{session_id}/check")))
            .json(&json!({}))
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(NoteDeckError::Auth(format!(
                "MiAuth check failed: {}",
                res.status().as_u16()
            )));
        }

        let text = Self::read_body_limited(res, "miauth/check").await?;
        let data: RawMiAuthResponse = serde_json::from_str(&text)?;
        if !data.ok {
            return Err(NoteDeckError::Auth(
                "MiAuth authentication was not completed".to_string(),
            ));
        }

        let token = data
            .token
            .ok_or_else(|| NoteDeckError::Auth("MiAuth response missing token".to_string()))?;
        let user = data
            .user
            .ok_or_else(|| NoteDeckError::Auth("MiAuth response missing user".to_string()))?;

        Ok(AuthResult {
            token,
            user: user.into(),
        })
    }

    /// Fetch all keys in a registry scope. Returns None if empty or not found (API error).
    /// Propagates network and other non-API errors.
    pub async fn get_registry_all(
        &self,
        host: &str,
        token: &str,
        scope: &[String],
    ) -> Result<Option<Value>, NoteDeckError> {
        let data = self
            .request(host, token, "i/registry/get-all", json!({ "scope": scope }))
            .await;
        match data {
            Ok(v) => {
                if let Some(obj) = v.as_object() {
                    if obj.is_empty() {
                        return Ok(None);
                    }
                }
                Ok(Some(v))
            }
            Err(NoteDeckError::Api { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn get_note_children(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        note_id: &str,
        limit: u32,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "notes/children",
                json!({ "noteId": note_id, "limit": limit }),
            )
            .await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn get_note_conversation(
        &self,
        host: &str,
        token: &str,
        account_id: &str,
        note_id: &str,
        limit: u32,
    ) -> Result<Vec<NormalizedNote>, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "notes/conversation",
                json!({ "noteId": note_id, "limit": limit }),
            )
            .await?;
        let raw: Vec<RawNote> = serde_json::from_value(data)?;
        Ok(raw
            .into_iter()
            .map(|n| n.normalize(account_id, host))
            .collect())
    }

    pub async fn lookup_user(
        &self,
        host: &str,
        token: &str,
        username: &str,
        user_host: Option<&str>,
    ) -> Result<NormalizedUser, NoteDeckError> {
        let mut params = json!({ "username": username });
        if let Some(h) = user_host {
            params["host"] = json!(h);
        }
        let data = self.request(host, token, "users/show", params).await?;
        let raw: RawUser = serde_json::from_value(data)?;
        Ok(raw.into())
    }

    pub async fn follow_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "following/create", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn unfollow_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "following/delete", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn accept_follow_request(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "following/requests/accept", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn reject_follow_request(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "following/requests/reject", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    /// Fetch server meta information.
    pub async fn get_meta(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "meta", json!({})).await
    }

    /// Fetch boolean policy flags and mode flags from /api/i.
    /// Returns policies (e.g. ltlAvailable, yamiTlAvailable) and
    /// top-level mode flags matching isIn*Mode (e.g. isInYamiMode).
    pub async fn get_user_policies(
        &self,
        host: &str,
        token: &str,
    ) -> Result<HashMap<String, bool>, NoteDeckError> {
        let data = self.request(host, token, "i", json!({})).await?;
        let mut result = HashMap::new();
        if let Some(policies) = data.get("policies").and_then(|v| v.as_object()) {
            for (key, value) in policies {
                if let Some(b) = value.as_bool() {
                    result.insert(key.clone(), b);
                }
            }
        }
        // Extract top-level mode flags (fork features like yami/hanami mode)
        if let Some(obj) = data.as_object() {
            for (key, value) in obj {
                if key.starts_with("isIn") && key.ends_with("Mode") {
                    if let Some(b) = value.as_bool() {
                        result.insert(key.clone(), b);
                    }
                }
            }
        }
        Ok(result)
    }

    /// Update a user setting via /api/i/update.
    pub async fn update_user_setting(
        &self,
        host: &str,
        token: &str,
        key: &str,
        value: bool,
    ) -> Result<(), NoteDeckError> {
        let mut params = json!({});
        params[key] = json!(value);
        self.request(host, token, "i/update", params).await?;
        Ok(())
    }

    /// Fetch parameter names for a specific API endpoint (public, no auth required).
    pub async fn get_endpoint_params(
        &self,
        host: &str,
        endpoint: &str,
    ) -> Result<Vec<String>, NoteDeckError> {
        let res = self
            .client
            .post(self.api_url(host, "endpoint"))
            .json(&json!({ "endpoint": endpoint }))
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(NoteDeckError::Api {
                endpoint: "endpoint".to_string(),
                status: res.status().as_u16(),
                message: "Failed to fetch endpoint info".to_string(),
            });
        }

        let text = Self::read_body_limited(res, "endpoint").await?;
        let data: Value = serde_json::from_str(&text)?;
        let mut params = Vec::new();

        // Misskey 2024+: params.properties is an object keyed by param name
        if let Some(props) = data
            .pointer("/params/properties")
            .and_then(|v| v.as_object())
        {
            for key in props.keys() {
                params.push(key.clone());
            }
        }
        // Older Misskey: params is a flat array with { name, ... } items
        if params.is_empty() {
            if let Some(arr) = data.get("params").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        params.push(name.to_string());
                    }
                }
            }
        }

        Ok(params)
    }

    /// Fetch enum values for a specific parameter of an endpoint.
    /// Fetch available API endpoints (public, no auth required).
    pub async fn get_endpoints(&self, host: &str) -> Result<Vec<String>, NoteDeckError> {
        let res = self
            .client
            .post(self.api_url(host, "endpoints"))
            .json(&json!({}))
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(NoteDeckError::Api {
                endpoint: "endpoints".to_string(),
                status: res.status().as_u16(),
                message: "Failed to fetch endpoints".to_string(),
            });
        }

        let text = Self::read_body_limited(res, "endpoints").await?;
        let endpoints: Vec<String> = serde_json::from_str(&text)?;
        Ok(endpoints)
    }

    // --- Notifications ---

    pub async fn get_unread_notification_count(
        &self,
        host: &str,
        token: &str,
    ) -> Result<i64, NoteDeckError> {
        let data = self
            .request(host, token, "notifications/unread-count", json!({}))
            .await?;
        Ok(data.get("count").and_then(|v| v.as_i64()).unwrap_or(0))
    }

    pub async fn mark_all_notifications_as_read(
        &self,
        host: &str,
        token: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "notifications/mark-all-as-read", json!({}))
            .await?;
        Ok(())
    }

    // --- Unread chat ---

    pub async fn get_unread_chat(
        &self,
        host: &str,
        token: &str,
    ) -> Result<bool, NoteDeckError> {
        let data = self
            .request(host, token, "messaging/unread", json!({}))
            .await?;
        Ok(data.as_bool().unwrap_or(false))
    }

    // --- Self (current user) ---

    pub async fn get_self(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "i", json!({})).await
    }

    // --- Drive ---

    pub async fn get_drive_folders(
        &self,
        host: &str,
        token: &str,
        folder_id: Option<&str>,
        limit: i64,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        if let Some(id) = folder_id {
            params["folderId"] = json!(id);
        }
        self.request(host, token, "drive/folders", params).await
    }

    pub async fn get_drive_files(
        &self,
        host: &str,
        token: &str,
        folder_id: Option<&str>,
        limit: i64,
        file_type: Option<&str>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        if let Some(id) = folder_id {
            params["folderId"] = json!(id);
        }
        if let Some(t) = file_type {
            params["type"] = json!(t);
        }
        self.request(host, token, "drive/files", params).await
    }

    pub async fn delete_drive_file(
        &self,
        host: &str,
        token: &str,
        file_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "drive/files/delete", json!({ "fileId": file_id }))
            .await?;
        Ok(())
    }

    // --- Follow requests ---

    pub async fn get_follow_requests(
        &self,
        host: &str,
        token: &str,
        limit: i64,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "following/requests/list", json!({ "limit": limit }))
            .await
    }

    // --- Explore (users/roles) ---

    pub async fn search_users(
        &self,
        host: &str,
        token: &str,
        opts: SearchUsersOptions<'_>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "limit": opts.limit });
        if let Some(q) = opts.query {
            params["query"] = json!(q);
        }
        if let Some(o) = opts.origin {
            params["origin"] = json!(o);
        }
        if let Some(s) = opts.sort {
            params["sort"] = json!(s);
        }
        if let Some(s) = opts.state {
            params["state"] = json!(s);
        }
        if let Some(o) = opts.offset {
            params["offset"] = json!(o);
        }
        self.request(host, token, "users", params).await
    }

    pub async fn get_roles(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "roles/list", json!({})).await
    }

    pub async fn get_role_users(
        &self,
        host: &str,
        token: &str,
        role_id: &str,
        limit: i64,
        offset: Option<i64>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "roleId": role_id, "limit": limit });
        if let Some(o) = offset {
            params["offset"] = json!(o);
        }
        self.request(host, token, "roles/users", params).await
    }

    // --- Announcements ---

    pub async fn get_announcements(
        &self,
        host: &str,
        token: &str,
        limit: i64,
        is_active: bool,
    ) -> Result<Value, NoteDeckError> {
        self.request(
            host,
            token,
            "announcements",
            json!({ "limit": limit, "isActive": is_active }),
        )
        .await
    }

    pub async fn read_announcement(
        &self,
        host: &str,
        token: &str,
        announcement_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "i/read-announcement",
            json!({ "announcementId": announcement_id }),
        )
        .await?;
        Ok(())
    }

    // --- Chat reactions ---

    pub async fn react_chat_message(
        &self,
        host: &str,
        token: &str,
        message_id: &str,
        reaction: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "chat/messages/react",
            json!({ "messageId": message_id, "reaction": reaction }),
        )
        .await?;
        Ok(())
    }

    pub async fn unreact_chat_message(
        &self,
        host: &str,
        token: &str,
        message_id: &str,
        reaction: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "chat/messages/unreact",
            json!({ "messageId": message_id, "reaction": reaction }),
        )
        .await?;
        Ok(())
    }

    // --- Search ---

    pub async fn search_users_by_query(
        &self,
        host: &str,
        token: &str,
        query: &str,
        limit: i64,
    ) -> Result<Value, NoteDeckError> {
        self.request(
            host,
            token,
            "users/search",
            json!({ "query": query, "limit": limit }),
        )
        .await
    }

    pub async fn search_hashtags(
        &self,
        host: &str,
        token: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<String>, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "hashtags/search",
                json!({ "query": query, "limit": limit }),
            )
            .await?;
        let tags: Vec<String> = serde_json::from_value(data)?;
        Ok(tags)
    }

    // --- ActivityPub resolve ---

    pub async fn ap_show(
        &self,
        host: &str,
        token: &str,
        uri: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "ap/show", json!({ "uri": uri }))
            .await
    }

    // --- Server stats ---

    pub async fn get_server_stats(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "stats", json!({})).await
    }

    pub async fn get_meta_detail(
        &self,
        host: &str,
        token: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "meta", json!({ "detail": true }))
            .await
    }

    // --- User achievements ---

    pub async fn get_user_achievements(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(
            host,
            token,
            "users/achievements",
            json!({ "userId": user_id }),
        )
        .await
    }

    // --- User notes (with filters) ---

    pub async fn get_user_notes_filtered(
        &self,
        host: &str,
        token: &str,
        params: Value,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "users/notes", params).await
    }

    pub async fn get_user_featured_notes(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        limit: i64,
        until_id: Option<&str>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "userId": user_id, "limit": limit });
        apply_pagination(&mut params, None, until_id);
        self.request(host, token, "users/featured-notes", params)
            .await
    }

    // --- Pages ---

    pub async fn get_pages(
        &self,
        host: &str,
        token: &str,
        endpoint: &str,
        limit: i64,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, endpoint, json!({ "limit": limit }))
            .await
    }

    pub async fn get_page(
        &self,
        host: &str,
        token: &str,
        page_id: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "pages/show", json!({ "pageId": page_id }))
            .await
    }

    pub async fn like_page(
        &self,
        host: &str,
        token: &str,
        page_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "pages/like", json!({ "pageId": page_id }))
            .await?;
        Ok(())
    }

    pub async fn unlike_page(
        &self,
        host: &str,
        token: &str,
        page_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "pages/unlike", json!({ "pageId": page_id }))
            .await?;
        Ok(())
    }

    // --- Gallery ---

    pub async fn get_gallery_posts(
        &self,
        host: &str,
        token: &str,
        limit: i64,
        until_id: Option<&str>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        apply_pagination(&mut params, None, until_id);
        self.request(host, token, "gallery/posts", params).await
    }

    pub async fn like_gallery_post(
        &self,
        host: &str,
        token: &str,
        post_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "gallery/posts/like",
            json!({ "postId": post_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn unlike_gallery_post(
        &self,
        host: &str,
        token: &str,
        post_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "gallery/posts/unlike",
            json!({ "postId": post_id }),
        )
        .await?;
        Ok(())
    }

    // --- Flash (Play) ---

    pub async fn get_flashes(
        &self,
        host: &str,
        token: &str,
        endpoint: &str,
        limit: i64,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, endpoint, json!({ "limit": limit }))
            .await
    }

    pub async fn get_flash(
        &self,
        host: &str,
        token: &str,
        flash_id: &str,
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "flash/show", json!({ "flashId": flash_id }))
            .await
    }

    pub async fn like_flash(
        &self,
        host: &str,
        token: &str,
        flash_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "flash/like", json!({ "flashId": flash_id }))
            .await?;
        Ok(())
    }

    pub async fn unlike_flash(
        &self,
        host: &str,
        token: &str,
        flash_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "flash/unlike", json!({ "flashId": flash_id }))
            .await?;
        Ok(())
    }

    // --- Chat API ---

    pub async fn get_chat_history(
        &self,
        host: &str,
        token: &str,
        limit: i64,
        room: bool,
    ) -> Result<Vec<ChatMessage>, NoteDeckError> {
        let mut params = json!({ "limit": limit });
        if room {
            params["room"] = json!(true);
        }
        let data = self
            .request(host, token, "chat/history", params)
            .await?;
        let messages: Vec<ChatMessage> = serde_json::from_value(data)?;
        Ok(messages)
    }

    pub async fn get_chat_user_messages(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<ChatMessage>, NoteDeckError> {
        let mut params = json!({
            "userId": user_id,
            "limit": limit,
        });
        apply_pagination(&mut params, since_id, until_id);
        let data = self
            .request(host, token, "chat/messages/user-timeline", params)
            .await?;
        let messages: Vec<ChatMessage> = serde_json::from_value(data)?;
        Ok(messages)
    }

    pub async fn get_chat_room_messages(
        &self,
        host: &str,
        token: &str,
        room_id: &str,
        limit: i64,
        since_id: Option<&str>,
        until_id: Option<&str>,
    ) -> Result<Vec<ChatMessage>, NoteDeckError> {
        let mut params = json!({
            "roomId": room_id,
            "limit": limit,
        });
        apply_pagination(&mut params, since_id, until_id);
        let data = self
            .request(host, token, "chat/messages/room-timeline", params)
            .await?;
        let messages: Vec<ChatMessage> = serde_json::from_value(data)?;
        Ok(messages)
    }

    pub async fn create_chat_message_to_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        text: &str,
    ) -> Result<ChatMessage, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "chat/messages/create-to-user",
                json!({ "userId": user_id, "text": text }),
            )
            .await?;
        let message: ChatMessage = serde_json::from_value(data)?;
        Ok(message)
    }

    pub async fn create_chat_message_to_room(
        &self,
        host: &str,
        token: &str,
        room_id: &str,
        text: &str,
    ) -> Result<ChatMessage, NoteDeckError> {
        let data = self
            .request(
                host,
                token,
                "chat/messages/create-to-room",
                json!({ "roomId": room_id, "text": text }),
            )
            .await?;
        let message: ChatMessage = serde_json::from_value(data)?;
        Ok(message)
    }

    // --- Legacy messaging ---

    pub async fn create_messaging_message(
        &self,
        host: &str,
        token: &str,
        params: Value,
    ) -> Result<ChatMessage, NoteDeckError> {
        let data = self
            .request(host, token, "messaging/messages/create", params)
            .await?;
        let message: ChatMessage = serde_json::from_value(data)?;
        Ok(message)
    }

    // --- Server Discovery (unauthenticated) ---

    /// Fetch nodeinfo via .well-known/nodeinfo.
    /// Returns the parsed nodeinfo JSON.
    pub async fn fetch_nodeinfo(&self, host: &str) -> Result<Value, NoteDeckError> {
        let well_known_url = format!("https://{host}/.well-known/nodeinfo");
        let res = self
            .client
            .get(&well_known_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(NoteDeckError::Api {
                endpoint: ".well-known/nodeinfo".to_string(),
                status: res.status().as_u16(),
                message: "Failed to fetch well-known nodeinfo".to_string(),
            });
        }
        let text = Self::read_body_limited(res, ".well-known/nodeinfo").await?;
        let well_known: Value = serde_json::from_str(&text)?;

        let nodeinfo_url = well_known["links"]
            .as_array()
            .and_then(|links| {
                links.iter().find_map(|link| {
                    let rel = link["rel"].as_str().unwrap_or("");
                    if rel.contains("nodeinfo") {
                        link["href"].as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| NoteDeckError::Api {
                endpoint: ".well-known/nodeinfo".to_string(),
                status: 0,
                message: format!("No nodeinfo URL found for {host}"),
            })?;

        // Validate URL to prevent SSRF: must be https://{host}/...
        let expected_prefix = format!("https://{host}/");
        if !nodeinfo_url.starts_with(&expected_prefix) {
            return Err(NoteDeckError::Api {
                endpoint: ".well-known/nodeinfo".to_string(),
                status: 0,
                message: format!("Nodeinfo URL host/scheme mismatch for {host}"),
            });
        }

        let res = self
            .client
            .get(&nodeinfo_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(NoteDeckError::Api {
                endpoint: "nodeinfo".to_string(),
                status: res.status().as_u16(),
                message: "Failed to fetch nodeinfo".to_string(),
            });
        }
        let text = Self::read_body_limited(res, "nodeinfo").await?;
        let nodeinfo: Value = serde_json::from_str(&text)?;
        Ok(nodeinfo)
    }

    // --- Pin/Unpin ---

    pub async fn pin_note(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "i/pin", json!({ "noteId": note_id }))
            .await?;
        Ok(())
    }

    pub async fn unpin_note(
        &self,
        host: &str,
        token: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "i/unpin", json!({ "noteId": note_id }))
            .await?;
        Ok(())
    }

    pub async fn get_user_pinned_note_ids(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Vec<String>, NoteDeckError> {
        let data = self
            .request(host, token, "users/show", json!({ "userId": user_id }))
            .await?;
        let ids = data
            .get("pinnedNoteIds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(ids)
    }

    // --- Mute/Block ---

    pub async fn mute_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "mute/create", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn unmute_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "mute/delete", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn block_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "blocking/create", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    pub async fn unblock_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(host, token, "blocking/delete", json!({ "userId": user_id }))
            .await?;
        Ok(())
    }

    // --- Report ---

    pub async fn report_user(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        comment: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "users/report-abuse",
            json!({ "userId": user_id, "comment": comment }),
        )
        .await?;
        Ok(())
    }

    // --- Clip operations ---

    pub async fn add_note_to_clip(
        &self,
        host: &str,
        token: &str,
        clip_id: &str,
        note_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "clips/add-note",
            json!({ "clipId": clip_id, "noteId": note_id }),
        )
        .await?;
        Ok(())
    }

    // --- User list operations ---

    pub async fn add_user_to_list(
        &self,
        host: &str,
        token: &str,
        list_id: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "users/lists/push",
            json!({ "listId": list_id, "userId": user_id }),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_user_from_list(
        &self,
        host: &str,
        token: &str,
        list_id: &str,
        user_id: &str,
    ) -> Result<(), NoteDeckError> {
        self.request(
            host,
            token,
            "users/lists/pull",
            json!({ "listId": list_id, "userId": user_id }),
        )
        .await?;
        Ok(())
    }

    // --- Follow list & relations ---

    pub async fn get_following(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        limit: i64,
        until_id: Option<&str>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "userId": user_id, "limit": limit });
        apply_pagination(&mut params, None, until_id);
        self.request(host, token, "users/following", params).await
    }

    pub async fn get_followers(
        &self,
        host: &str,
        token: &str,
        user_id: &str,
        limit: i64,
        until_id: Option<&str>,
    ) -> Result<Value, NoteDeckError> {
        let mut params = json!({ "userId": user_id, "limit": limit });
        apply_pagination(&mut params, None, until_id);
        self.request(host, token, "users/followers", params).await
    }

    pub async fn get_user_relations(
        &self,
        host: &str,
        token: &str,
        user_ids: &[String],
    ) -> Result<Value, NoteDeckError> {
        self.request(host, token, "users/relation", json!({ "userId": user_ids }))
            .await
    }

    // --- Server Discovery (unauthenticated) ---

    /// Fetch server meta (icon URL) via /api/meta (unauthenticated).
    pub async fn fetch_server_meta(&self, host: &str) -> Result<Value, NoteDeckError> {
        let url = self.api_url(host, "meta");
        let res = self
            .client
            .post(&url)
            .json(&json!({}))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(NoteDeckError::Api {
                endpoint: "meta".to_string(),
                status: res.status().as_u16(),
                message: "Failed to fetch server meta".to_string(),
            });
        }
        let text = Self::read_body_limited(res, "meta").await?;
        let meta: Value = serde_json::from_str(&text)?;
        Ok(meta)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn raw_note_json(id: &str, text: &str) -> Value {
        json!({
            "id": id,
            "createdAt": "2025-01-01T00:00:00.000Z",
            "text": text,
            "cw": null,
            "user": {"id": "u1", "username": "testuser"},
            "visibility": "public",
            "poll": null,
            "replyId": null,
            "renoteId": null,
            "channelId": null,
            "reactionAcceptance": null,
            "uri": null,
            "url": null,
            "updatedAt": null,
            "reply": null,
            "renote": null,
            "myReaction": null
        })
    }

    #[tokio::test]
    async fn request_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/test/endpoint"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let result = client
            .request("unused", "token", "test/endpoint", json!({}))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);
    }

    #[tokio::test]
    async fn request_includes_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/i"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let _ = client.request("h", "my-secret-token", "i", json!({})).await;

        // Verify the mock was hit (if token wasn't injected into body, request would still succeed)
    }

    #[tokio::test]
    async fn request_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/show"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"error": {"message": "No such note", "code": "NO_SUCH_NOTE"}})),
            )
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let err = client
            .request("h", "token", "notes/show", json!({"noteId": "n1"}))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "API");
        assert!(err.to_string().contains("No such note"));
    }

    #[tokio::test]
    async fn request_api_error_without_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/test"))
            .respond_with(ResponseTemplate::new(500).set_body_string(""))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let err = client
            .request("h", "token", "test", json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "API");
        // Falls back to endpoint (status)
        assert!(err.to_string().contains("test"));
    }

    #[tokio::test]
    async fn request_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/delete"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let result = client
            .request("h", "token", "notes/delete", json!({}))
            .await
            .unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn get_timeline_parses_notes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/timeline"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([
                        raw_note_json("n1", "Hello"),
                        raw_note_json("n2", "World"),
                    ])),
            )
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let notes = client
            .get_timeline(
                "h",
                "token",
                "acc1",
                TimelineType::new("home"),
                TimelineOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].id, "n1");
        assert_eq!(notes[0].text.as_deref(), Some("Hello"));
        assert_eq!(notes[0].account_id, "acc1");
        assert_eq!(notes[1].id, "n2");
    }

    #[tokio::test]
    async fn get_note_returns_normalized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(raw_note_json("n1", "test")))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let note = client.get_note("h", "token", "acc1", "n1").await.unwrap();
        assert_eq!(note.id, "n1");
        assert_eq!(note.server_host, "h");
    }

    #[tokio::test]
    async fn create_note_parses_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/create"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"createdNote": raw_note_json("n1", "posted")})),
            )
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let params = CreateNoteParams {
            text: Some("posted".into()),
            cw: None,
            visibility: Some("public".into()),
            local_only: None,
            mode_flags: None,
            reply_id: None,
            renote_id: None,
            file_ids: None,
            poll: None,
            scheduled_at: None,
        };
        let note = client
            .create_note("h", "token", "acc1", params)
            .await
            .unwrap();
        assert_eq!(note.id, "n1");
        assert_eq!(note.text.as_deref(), Some("posted"));
    }

    #[tokio::test]
    async fn delete_note_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/delete"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        client.delete_note("h", "token", "n1").await.unwrap();
    }

    #[tokio::test]
    async fn create_reaction_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/reactions/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        client
            .create_reaction("h", "token", "n1", ":star:")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_notifications_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/i/notifications"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": "notif1",
                "createdAt": "2025-01-01T00:00:00.000Z",
                "type": "reaction",
                "user": {"id": "u1", "username": "taka"},
                "note": raw_note_json("n1", "test"),
                "reaction": ":star:"
            }])))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let notifs = client
            .get_notifications("h", "token", "acc1", TimelineOptions::default())
            .await
            .unwrap();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].notification_type, "reaction");
        assert_eq!(notifs[0].reaction.as_deref(), Some(":star:"));
    }

    #[tokio::test]
    async fn search_notes_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([raw_note_json("n1", "Rust is great")])),
            )
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let notes = client
            .search_notes("h", "token", "acc1", "Rust", SearchOptions::default())
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text.as_deref(), Some("Rust is great"));
    }

    #[tokio::test]
    async fn get_user_detail_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/users/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "u1",
                "username": "taka",
                "host": null,
                "name": "Taka",
                "avatarUrl": null,
                "bannerUrl": null,
                "description": "hello",
                "followersCount": 10,
                "followingCount": 5,
                "notesCount": 100,
                "createdAt": "2024-01-01T00:00:00.000Z"
            })))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let user = client.get_user_detail("h", "token", "u1").await.unwrap();
        assert_eq!(user.username, "taka");
        assert_eq!(user.followers_count, 10);
        assert_eq!(user.notes_count, 100);
    }

    #[tokio::test]
    async fn get_server_emojis_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/emojis"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "emojis": [
                    {"name": "blobcat", "url": "https://example.com/blobcat.png", "category": "blob", "aliases": ["neko"]}
                ]
            })))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let emojis = client.get_server_emojis("h", "token").await.unwrap();
        assert_eq!(emojis.len(), 1);
        assert_eq!(emojis[0].name, "blobcat");
        assert_eq!(emojis[0].aliases, vec!["neko"]);
    }

    #[tokio::test]
    async fn get_user_lists_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/users/lists/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": "l1", "name": "Friends"},
                {"id": "l2", "name": "Tech"}
            ])))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let lists = client.get_user_lists("h", "token").await.unwrap();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].name, "Friends");
    }

    #[tokio::test]
    async fn get_favorites_unwraps_note_field() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/i/favorites"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": "fav1", "note": raw_note_json("n1", "fav note")}
            ])))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let notes = client
            .get_favorites("h", "token", "acc1", 20, None, None)
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "n1");
        assert_eq!(notes[0].text.as_deref(), Some("fav note"));
    }

    #[tokio::test]
    async fn follow_user_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/following/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        client.follow_user("h", "token", "u1").await.unwrap();
    }

    #[tokio::test]
    async fn extract_reaction_palette_valid() {
        let data = json!([
            [["client", "preferences", "sync"], [{"id": "default", "name": "Default", "emojis": [":star:", ":heart:", ":thumbsup:"]}]]
        ]);
        let result = MisskeyClient::extract_reaction_palette(&data).unwrap();
        assert_eq!(result, vec![":star:", ":heart:", ":thumbsup:"]);
    }

    #[tokio::test]
    async fn extract_reaction_palette_empty() {
        let data = json!([]);
        assert!(MisskeyClient::extract_reaction_palette(&data).is_none());
    }

    #[tokio::test]
    async fn get_note_children_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notes/children"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([raw_note_json("r1", "reply")])),
            )
            .mount(&server)
            .await;

        let client = MisskeyClient::with_base_url(&server.uri());
        let notes = client
            .get_note_children("h", "token", "acc1", "n1", 20)
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "r1");
    }
}
