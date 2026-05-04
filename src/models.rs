use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use zeroize::Zeroize;

fn deserialize_nullable_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

// --- DB models ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub host: String,
    pub token: String,
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub software: String,
}

impl Drop for Account {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

/// Token を含まない、フロントエンド向け Account 構造体
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AccountPublic {
    pub id: String,
    pub host: String,
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub software: String,
    pub has_token: bool,
}

impl AccountPublic {
    pub fn new(a: &Account, has_token: bool) -> Self {
        Self {
            id: a.id.clone(),
            host: a.host.clone(),
            user_id: a.user_id.clone(),
            username: a.username.clone(),
            display_name: a.display_name.clone(),
            avatar_url: a.avatar_url.clone(),
            software: a.software.clone(),
            has_token,
        }
    }
}

impl From<&Account> for AccountPublic {
    fn from(a: &Account) -> Self {
        Self::new(a, !a.token.is_empty())
    }
}

impl From<Account> for AccountPublic {
    fn from(mut a: Account) -> Self {
        let has_token = !a.token.is_empty();
        // Zeroize token before taking fields via clone to maintain security invariant
        a.token.zeroize();
        Self {
            id: std::mem::take(&mut a.id),
            host: std::mem::take(&mut a.host),
            user_id: std::mem::take(&mut a.user_id),
            username: std::mem::take(&mut a.username),
            display_name: a.display_name.take(),
            avatar_url: a.avatar_url.take(),
            software: std::mem::take(&mut a.software),
            has_token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StoredServer {
    pub host: String,
    pub software: String,
    pub version: String,
    pub features_json: String,
    pub updated_at: i64,
}

// --- Normalized models (sent to frontend via IPC) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedNote {
    pub id: String,
    #[serde(rename = "_accountId")]
    pub account_id: String,
    #[serde(rename = "_serverHost")]
    pub server_host: String,
    pub created_at: String,
    pub text: Option<String>,
    pub cw: Option<String>,
    pub user: NormalizedUser,
    pub visibility: String,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    #[serde(default)]
    pub reaction_emojis: HashMap<String, String>,
    #[serde(default)]
    pub reactions: HashMap<String, i64>,
    pub my_reaction: Option<String>,
    pub renote_count: i64,
    pub replies_count: i64,
    #[serde(default)]
    pub files: Vec<NormalizedDriveFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll: Option<NormalizedPoll>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_acceptance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default)]
    pub visible_user_ids: Vec<String>,
    #[serde(default)]
    pub is_favorited: bool,
    /// Fork-specific mode flags (e.g., isNoteInYamiMode)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mode_flags: HashMap<String, bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(type = Option<Box<serde_json::Value>>))]
    pub reply: Option<Box<NormalizedNote>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(type = Option<Box<serde_json::Value>>))]
    pub renote: Option<Box<NormalizedNote>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AvatarDecoration {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub angle: Option<f64>,
    #[serde(default)]
    pub flip_h: Option<bool>,
    #[serde(default)]
    pub offset_x: Option<f64>,
    #[serde(default)]
    pub offset_y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserInstance {
    pub name: Option<String>,
    pub favicon_url: Option<String>,
    pub icon_url: Option<String>,
    pub theme_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedUser {
    pub id: String,
    pub username: String,
    pub host: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub is_cat: bool,
    #[serde(default)]
    pub avatar_decorations: Vec<AvatarDecoration>,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<UserInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserRole {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub icon_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub display_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct UserField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedUserDetail {
    pub id: String,
    pub username: String,
    pub host: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub followers_count: i64,
    #[serde(default)]
    pub following_count: i64,
    #[serde(default)]
    pub notes_count: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub is_cat: bool,
    #[serde(default)]
    pub is_following: bool,
    #[serde(default)]
    pub is_followed: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub avatar_decorations: Vec<AvatarDecoration>,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    #[serde(default)]
    pub roles: Vec<UserRole>,
    #[serde(default)]
    pub fields: Vec<UserField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following_visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers_visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPoll {
    pub choices: Vec<NormalizedPollChoice>,
    #[serde(default)]
    pub multiple: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPollChoice {
    pub text: String,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub is_voted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedDriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub url: String,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub is_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReactionInfo {
    pub user: NormalizedUser,
    pub reaction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedNotification {
    pub id: String,
    #[serde(rename = "_accountId")]
    pub account_id: String,
    #[serde(rename = "_serverHost")]
    pub server_host: String,
    pub created_at: String,
    #[serde(rename = "type")]
    pub notification_type: String,
    pub user: Option<NormalizedUser>,
    pub note: Option<NormalizedNote>,
    pub reaction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievement: Option<String>,
    /// Grouped reactions (for reaction:grouped type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionInfo>>,
    /// Grouped users (for renote:grouped type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<NormalizedUser>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteParams {
    pub text: Option<String>,
    pub cw: Option<String>,
    pub visibility: Option<String>,
    pub local_only: Option<bool>,
    pub mode_flags: Option<HashMap<String, bool>>,
    pub reply_id: Option<String>,
    pub renote_id: Option<String>,
    pub file_ids: Option<Vec<String>>,
    pub poll: Option<CreateNotePoll>,
    pub scheduled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CreateNotePoll {
    pub choices: Vec<String>,
    pub multiple: Option<bool>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct TimelineType(String);

impl TimelineType {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn api_endpoint(&self) -> String {
        match self.0.as_str() {
            "home" => "notes/timeline".to_string(),
            "local" => "notes/local-timeline".to_string(),
            "social" => "notes/hybrid-timeline".to_string(),
            "global" => "notes/global-timeline".to_string(),
            other => format!("notes/{other}-timeline"),
        }
    }

    pub fn ws_channel(&self) -> String {
        match self.0.as_str() {
            "home" => "homeTimeline".to_string(),
            "local" => "localTimeline".to_string(),
            "social" => "hybridTimeline".to_string(),
            "global" => "globalTimeline".to_string(),
            other => format!("{other}Timeline"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TimelineFilter {
    pub with_renotes: Option<bool>,
    pub with_replies: Option<bool>,
    pub with_files: Option<bool>,
    pub with_bots: Option<bool>,
    pub with_sensitive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TimelineOptions {
    #[serde(default = "default_limit")]
    limit: i64,
    pub since_id: Option<String>,
    pub until_id: Option<String>,
    #[serde(default)]
    pub filters: Option<TimelineFilter>,
    pub list_id: Option<String>,
}

impl TimelineOptions {
    pub fn new(limit: i64, since_id: Option<String>, until_id: Option<String>) -> Self {
        Self {
            limit,
            since_id,
            until_id,
            filters: None,
            list_id: None,
        }
    }

    /// Returns limit clamped to 1..=100
    pub fn limit(&self) -> i64 {
        self.limit.clamp(1, 100)
    }
}

impl Default for TimelineOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            since_id: None,
            until_id: None,
            filters: None,
            list_id: None,
        }
    }
}

fn default_limit() -> i64 {
    20
}

/// Misskey `users/lists/*` (list, show) の共通レスポンス。本家 schema
/// (packages/backend/src/models/json-schema/user-list.ts) に準拠。
///
/// `forPublic=true` で他人の公開リストを取得した時のみ `isLiked` /
/// `likedCount` が付加される (Clips の `isFavorited` / `favoritedCount` と
/// 非対称な命名は本家準拠)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserList {
    pub id: String,
    pub name: String,
    pub is_public: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_liked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liked_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Antenna {
    pub id: String,
    pub name: String,
}

// =============================================================================
// Misskey users 系個別エンドポイント (users/reactions, users/pages,
// users/flashs, users/gallery/posts)。本家 schema 準拠。
// =============================================================================

/// `users/reactions` がレスポンス内で含む note への薄い参照。
///
/// users/reactions のレスポンスは raw Misskey note schema を含むが、
/// notecli の NormalizedNote は `RawNote.normalize(account_id, host)` を経て
/// 構築する独自モデルなので直接デシリアライズできない。
/// notedeck 側ではこの id を使って adapter 経由で再取得・正規化する設計のため、
/// id のみ抜き出す。サーバーから来る他のフィールドは serde の default で破棄。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserReactionNoteRef {
    pub id: String,
}

/// `users/reactions` の 1 件分。自分のプロフィールで「リアクション」タブを
/// 開いたときに、自分が付けたリアクションとその対象 note を一覧する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserReaction {
    pub id: String,
    pub created_at: String,
    /// `type` は Rust 予約語。サーバーの JSON キーは "type"。
    #[serde(rename = "type")]
    pub reaction_type: String,
    pub note: UserReactionNoteRef,
}

/// `users/pages` / `pages/show` の 1 件分。本家 packages/backend/src/models/Page.ts。
/// プロフィール一覧で使うのは title / summary / createdAt のみだが、
/// `pages/show` でも同型を使えるようフルセットで定義。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub name: String,
    pub summary: Option<String>,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<NormalizedUser>,
    /// `content` / `variables` はブロック構造で複雑。生 JSON で運ぶ。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default)]
    pub align_center: bool,
    #[serde(default)]
    pub hide_title_when_pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye_catching_image_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye_catching_image: Option<NormalizedDriveFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liked_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_liked: Option<bool>,
}

/// `users/flashs` / `flash/show` の 1 件分。本家
/// packages/backend/src/models/Flash.ts。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Flash {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub summary: String,
    pub script: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<NormalizedUser>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liked_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_liked: Option<bool>,
}

/// `users/gallery/posts` / `gallery/posts/show` の 1 件分。本家
/// packages/backend/src/models/GalleryPost.ts。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GalleryPost {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub description: Option<String>,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<NormalizedUser>,
    pub files: Vec<NormalizedDriveFile>,
    #[serde(default)]
    pub is_sensitive: bool,
    #[serde(default)]
    pub liked_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_liked: Option<bool>,
}

// =============================================================================
// Misskey `charts/*` レスポンス。9 種類のエンドポイントごとに別構造体。
// 各フィールドは時系列データ点 (新→古順、index 0 = 今日) を i64 配列で持つ。
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserNotesChartDiffs {
    pub normal: Vec<i64>,
    pub reply: Vec<i64>,
    pub renote: Vec<i64>,
    pub with_file: Vec<i64>,
}

/// `charts/user/notes`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserNotesChart {
    pub inc: Vec<i64>,
    pub dec: Vec<i64>,
    pub diffs: UserNotesChartDiffs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FollowChartGroup {
    pub inc: Vec<i64>,
    pub dec: Vec<i64>,
    pub total: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FollowChartSection {
    pub followings: FollowChartGroup,
    pub followers: FollowChartGroup,
}

/// `charts/user/following`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserFollowingChart {
    pub local: FollowChartSection,
    pub remote: FollowChartSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PvChartGroup {
    pub user: Vec<i64>,
    pub visitor: Vec<i64>,
}

/// `charts/user/pv` (pv = Natural PV、upv = Unique PV)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UserPvChart {
    pub pv: PvChartGroup,
    pub upv: PvChartGroup,
}

/// `charts/active-users`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ActiveUsersChart {
    pub read_write: Vec<i64>,
    pub read: Vec<i64>,
    pub write: Vec<i64>,
    pub registered_within_week: Vec<i64>,
    pub registered_within_month: Vec<i64>,
    pub registered_within_year: Vec<i64>,
    pub registered_outside_week: Vec<i64>,
    pub registered_outside_month: Vec<i64>,
    pub registered_outside_year: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerNotesChartSection {
    pub total: Vec<i64>,
    pub inc: Vec<i64>,
    pub dec: Vec<i64>,
    pub diffs: UserNotesChartDiffs,
}

/// `charts/notes`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerNotesChart {
    pub local: ServerNotesChartSection,
    pub remote: ServerNotesChartSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerUsersChartSection {
    pub total: Vec<i64>,
    pub inc: Vec<i64>,
    pub dec: Vec<i64>,
}

/// `charts/users`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerUsersChart {
    pub local: ServerUsersChartSection,
    pub remote: ServerUsersChartSection,
}

/// `charts/federation`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FederationChart {
    pub delivered_instances: Vec<i64>,
    pub inbox_instances: Vec<i64>,
    pub stalled: Vec<i64>,
    pub sub: Vec<i64>,
    /// `pub` は Rust 予約語。サーバーの JSON キーは "pub"。
    #[serde(rename = "pub")]
    pub pub_: Vec<i64>,
    pub pubsub: Vec<i64>,
    pub sub_active: Vec<i64>,
    #[serde(rename = "pubActive")]
    pub pub_active: Vec<i64>,
}

/// `charts/ap-request` (ActivityPub の配送成功/失敗/受信数)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ApRequestChart {
    pub deliver_succeeded: Vec<i64>,
    pub deliver_failed: Vec<i64>,
    pub inbox_received: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerDriveChartSection {
    pub inc_count: Vec<i64>,
    pub inc_size: Vec<i64>,
    pub dec_count: Vec<i64>,
    pub dec_size: Vec<i64>,
}

/// `charts/drive` (Size は KB 単位)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ServerDriveChart {
    pub local: ServerDriveChartSection,
    pub remote: ServerDriveChartSection,
}

/// Misskey `notes/drafts/*` (2025.6+) のレスポンス。`notes/drafts/list` は
/// `Vec<NoteDraft>`、`notes/drafts/create` は `{ createdDraft: NoteDraft }`、
/// `notes/drafts/update` は `{ updatedDraft: NoteDraft }` を返す
/// (notedeck 側でラッパーを剥がして直接 NoteDraft を渡す)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NoteDraftPoll {
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NoteDraft {
    pub id: String,
    pub created_at: String,
    pub text: Option<String>,
    pub cw: Option<String>,
    pub visibility: String,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default)]
    pub file_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hashtag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<NoteDraftPoll>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<i64>,
    #[serde(default)]
    pub is_actually_scheduled: bool,
}

/// Misskey `clips/*` (clips/list, clips/show, clips/create, users/clips,
/// clips/my-favorites) の共通レスポンス。本家 schema
/// (packages/backend/src/models/json-schema/clip.ts) に準拠。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub created_at: String,
    pub last_clipped_at: Option<String>,
    pub user_id: String,
    pub user: NormalizedUser,
    pub name: String,
    pub description: Option<String>,
    pub is_public: bool,
    pub favorited_count: i64,
    /// `isFavorited` はログイン時のみサーバーから返る。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_favorited: Option<bool>,
    /// `notesCount` は一部エンドポイントのみ返る。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub created_at: String,
    pub from_user_id: String,
    pub from_user: Option<ChatUser>,
    pub to_user_id: Option<String>,
    pub to_user: Option<ChatUser>,
    pub to_room_id: Option<String>,
    pub to_room: Option<ChatRoom>,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file: Option<NormalizedDriveFile>,
    pub is_read: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_nullable_vec")]
    pub reactions: Vec<ChatMessageReaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageReaction {
    pub user: Option<ChatReactionUser>,
    pub reaction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ChatReactionUser {
    pub id: String,
    pub name: Option<String>,
    pub username: String,
    pub host: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ChatUser {
    pub id: String,
    pub name: Option<String>,
    pub username: String,
    pub host: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ChatRoom {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    #[serde(default = "default_limit")]
    limit: i64,
    pub since_id: Option<String>,
    pub until_id: Option<String>,
    pub since_date: Option<i64>,
    pub until_date: Option<i64>,
}

impl SearchOptions {
    pub fn new(limit: i64) -> Self {
        Self {
            limit,
            since_id: None,
            until_id: None,
            since_date: None,
            until_date: None,
        }
    }

    pub fn limit(&self) -> i64 {
        self.limit.clamp(1, 100)
    }
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            since_id: None,
            until_id: None,
            since_date: None,
            until_date: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub session_id: String,
    pub url: String,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AuthResult {
    pub token: String,
    pub user: NormalizedUser,
}

// --- Raw Misskey API response types (for deserialization) ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNote {
    pub id: String,
    pub created_at: String,
    pub text: Option<String>,
    pub cw: Option<String>,
    pub user: RawUser,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    #[serde(default)]
    pub reaction_emojis: HashMap<String, String>,
    #[serde(default)]
    pub reactions: HashMap<String, i64>,
    pub my_reaction: Option<String>,
    #[serde(default)]
    pub renote_count: i64,
    #[serde(default)]
    pub replies_count: i64,
    #[serde(default)]
    pub files: Vec<RawDriveFile>,
    pub poll: Option<RawPoll>,
    pub reply_id: Option<String>,
    pub renote_id: Option<String>,
    pub channel_id: Option<String>,
    pub channel: Option<Channel>,
    pub reaction_acceptance: Option<String>,
    pub uri: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default)]
    pub visible_user_ids: Vec<String>,
    #[serde(default)]
    pub is_favorited: bool,
    pub reply: Option<Box<RawNote>>,
    pub renote: Option<Box<RawNote>>,
    /// Catch-all for fork-specific fields (e.g., isNoteInYamiMode)
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUser {
    pub id: String,
    pub username: String,
    pub host: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub is_cat: bool,
    #[serde(default)]
    pub avatar_decorations: Vec<AvatarDecoration>,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    pub instance: Option<UserInstance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPoll {
    pub choices: Vec<RawPollChoice>,
    #[serde(default)]
    pub multiple: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPollChoice {
    pub text: String,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub is_voted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub url: String,
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub is_sensitive: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawReactionInfo {
    pub user: RawUser,
    pub reaction: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNotification {
    pub id: String,
    pub created_at: String,
    #[serde(rename = "type")]
    pub notification_type: String,
    pub user: Option<RawUser>,
    pub note: Option<RawNote>,
    pub reaction: Option<String>,
    pub message: Option<String>,
    pub achievement: Option<String>,
    /// Grouped reactions (for reaction:grouped type from notifications-grouped API)
    pub reactions: Option<Vec<RawReactionInfo>>,
    /// Grouped users (for renote:grouped type from notifications-grouped API)
    pub users: Option<Vec<RawUser>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUserRole {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub icon_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub display_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct RawUserField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUserDetail {
    pub id: String,
    pub username: String,
    pub host: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub followers_count: i64,
    #[serde(default)]
    pub following_count: i64,
    #[serde(default)]
    pub notes_count: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub is_cat: bool,
    #[serde(default)]
    pub is_following: bool,
    #[serde(default)]
    pub is_followed: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub avatar_decorations: Vec<AvatarDecoration>,
    #[serde(default)]
    pub emojis: HashMap<String, String>,
    #[serde(default)]
    pub roles: Vec<RawUserRole>,
    #[serde(default)]
    pub fields: Vec<RawUserField>,
    pub url: Option<String>,
    pub birthday: Option<String>,
    pub location: Option<String>,
    pub online_status: Option<String>,
    #[serde(default)]
    pub following_visibility: Option<String>,
    #[serde(default)]
    pub followers_visibility: Option<String>,
    pub followed_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawMiAuthResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub user: Option<RawUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawCreateNoteResponse {
    pub created_note: RawNote,
}

#[derive(Debug, Deserialize)]
pub struct RawEmojisResponse {
    pub emojis: Vec<RawEmoji>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEmoji {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Emoji info exposed to the frontend via Tauri commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ServerEmoji {
    pub name: String,
    pub url: String,
    pub category: Option<String>,
    pub aliases: Vec<String>,
}

impl From<RawEmoji> for ServerEmoji {
    fn from(raw: RawEmoji) -> Self {
        Self {
            name: raw.name,
            url: raw.url,
            category: raw.category,
            aliases: raw.aliases,
        }
    }
}

// --- Conversion: Raw -> Normalized ---

impl RawNote {
    pub fn normalize(self, account_id: &str, server_host: &str) -> NormalizedNote {
        NormalizedNote {
            id: self.id,
            account_id: account_id.to_string(),
            server_host: server_host.to_string(),
            created_at: self.created_at,
            text: self.text,
            cw: self.cw,
            user: self.user.into(),
            visibility: self.visibility,
            emojis: self.emojis,
            reaction_emojis: self.reaction_emojis,
            reactions: self.reactions,
            my_reaction: self.my_reaction,
            renote_count: self.renote_count,
            replies_count: self.replies_count,
            files: self.files.into_iter().map(Into::into).collect(),
            poll: self.poll.map(|p| NormalizedPoll {
                choices: p
                    .choices
                    .into_iter()
                    .map(|c| NormalizedPollChoice {
                        text: c.text,
                        votes: c.votes,
                        is_voted: c.is_voted,
                    })
                    .collect(),
                multiple: p.multiple,
                expires_at: p.expires_at,
            }),
            reply_id: self.reply_id,
            renote_id: self.renote_id,
            channel_id: self.channel_id,
            channel: self.channel,
            reaction_acceptance: self.reaction_acceptance,
            uri: self.uri,
            url: self.url,
            updated_at: self.updated_at,
            local_only: self.local_only,
            visible_user_ids: self.visible_user_ids,
            is_favorited: self.is_favorited,
            mode_flags: self
                .extra
                .into_iter()
                .filter(|(k, _)| k.starts_with("isNoteIn") && k.ends_with("Mode"))
                .filter_map(|(k, v)| v.as_bool().map(|b| (k, b)))
                .collect(),
            reply: self
                .reply
                .map(|r| Box::new(r.normalize(account_id, server_host))),
            renote: self
                .renote
                .map(|r| Box::new(r.normalize(account_id, server_host))),
        }
    }
}

// --- Note reaction (who reacted) ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNoteReaction {
    pub id: String,
    pub created_at: String,
    pub user: RawUser,
    #[serde(rename = "type")]
    pub reaction_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NormalizedNoteReaction {
    pub id: String,
    pub created_at: String,
    pub user: NormalizedUser,
    #[serde(rename = "type")]
    pub reaction_type: String,
}

impl From<RawNoteReaction> for NormalizedNoteReaction {
    fn from(r: RawNoteReaction) -> Self {
        Self {
            id: r.id,
            created_at: r.created_at,
            user: r.user.into(),
            reaction_type: r.reaction_type,
        }
    }
}

impl From<RawUser> for NormalizedUser {
    fn from(user: RawUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            host: user.host,
            name: user.name,
            avatar_url: user.avatar_url,
            is_bot: user.is_bot,
            is_cat: user.is_cat,
            avatar_decorations: user.avatar_decorations,
            emojis: user.emojis,
            instance: user.instance,
        }
    }
}

impl From<RawDriveFile> for NormalizedDriveFile {
    fn from(file: RawDriveFile) -> Self {
        Self {
            id: file.id,
            name: file.name,
            file_type: file.file_type,
            url: file.url,
            thumbnail_url: file.thumbnail_url,
            size: file.size,
            is_sensitive: file.is_sensitive,
        }
    }
}

impl RawUserDetail {
    pub fn normalize(self) -> NormalizedUserDetail {
        NormalizedUserDetail {
            id: self.id,
            username: self.username,
            host: self.host,
            name: self.name,
            avatar_url: self.avatar_url,
            banner_url: self.banner_url,
            description: self.description,
            followers_count: self.followers_count,
            following_count: self.following_count,
            notes_count: self.notes_count,
            is_bot: self.is_bot,
            is_cat: self.is_cat,
            is_following: self.is_following,
            is_followed: self.is_followed,
            created_at: self.created_at,
            avatar_decorations: self.avatar_decorations,
            emojis: self.emojis,
            roles: self
                .roles
                .into_iter()
                .map(|r| UserRole {
                    id: r.id,
                    name: r.name,
                    color: r.color,
                    icon_url: r.icon_url,
                    description: r.description,
                    display_order: r.display_order,
                })
                .collect(),
            fields: self
                .fields
                .into_iter()
                .map(|f| UserField {
                    name: f.name,
                    value: f.value,
                })
                .collect(),
            url: self.url,
            birthday: self.birthday,
            location: self.location,
            online_status: self.online_status,
            following_visibility: self.following_visibility,
            followers_visibility: self.followers_visibility,
            followed_message: self.followed_message,
        }
    }
}

impl RawNotification {
    pub fn normalize(self, account_id: &str, server_host: &str) -> NormalizedNotification {
        NormalizedNotification {
            id: self.id,
            account_id: account_id.to_string(),
            server_host: server_host.to_string(),
            created_at: self.created_at,
            notification_type: self.notification_type,
            user: self.user.map(Into::into),
            note: self.note.map(|n| n.normalize(account_id, server_host)),
            reaction: self.reaction,
            message: self.message,
            achievement: self.achievement,
            reactions: self.reactions.map(|rs| {
                rs.into_iter()
                    .map(|r| ReactionInfo {
                        user: r.user.into(),
                        reaction: r.reaction,
                    })
                    .collect()
            }),
            users: self
                .users
                .map(|us| us.into_iter().map(Into::into).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- TimelineType ----

    #[test]
    fn timeline_type_api_endpoint_known() {
        assert_eq!(TimelineType::new("home").api_endpoint(), "notes/timeline");
        assert_eq!(
            TimelineType::new("local").api_endpoint(),
            "notes/local-timeline"
        );
        assert_eq!(
            TimelineType::new("social").api_endpoint(),
            "notes/hybrid-timeline"
        );
        assert_eq!(
            TimelineType::new("global").api_endpoint(),
            "notes/global-timeline"
        );
    }

    #[test]
    fn timeline_type_api_endpoint_unknown_fallback() {
        assert_eq!(
            TimelineType::new("bubble").api_endpoint(),
            "notes/bubble-timeline"
        );
    }

    #[test]
    fn timeline_type_ws_channel_known() {
        assert_eq!(TimelineType::new("home").ws_channel(), "homeTimeline");
        assert_eq!(TimelineType::new("local").ws_channel(), "localTimeline");
        assert_eq!(TimelineType::new("social").ws_channel(), "hybridTimeline");
        assert_eq!(TimelineType::new("global").ws_channel(), "globalTimeline");
    }

    #[test]
    fn timeline_type_ws_channel_unknown_fallback() {
        assert_eq!(TimelineType::new("bubble").ws_channel(), "bubbleTimeline");
    }

    #[test]
    fn timeline_type_as_str() {
        let tt = TimelineType::new("home");
        assert_eq!(tt.as_str(), "home");
    }

    #[test]
    fn timeline_type_serde_roundtrip() {
        let tt = TimelineType::new("local");
        let json = serde_json::to_string(&tt).unwrap();
        assert_eq!(json, "\"local\"");
        let back: TimelineType = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "local");
    }

    // ---- TimelineOptions ----

    #[test]
    fn timeline_options_limit_clamp() {
        assert_eq!(TimelineOptions::new(0, None, None).limit(), 1);
        assert_eq!(TimelineOptions::new(-5, None, None).limit(), 1);
        assert_eq!(TimelineOptions::new(50, None, None).limit(), 50);
        assert_eq!(TimelineOptions::new(200, None, None).limit(), 100);
    }

    #[test]
    fn timeline_options_default() {
        let opts = TimelineOptions::default();
        assert_eq!(opts.limit(), 20);
        assert!(opts.since_id.is_none());
        assert!(opts.until_id.is_none());
        assert!(opts.filters.is_none());
        assert!(opts.list_id.is_none());
    }

    #[test]
    fn timeline_options_deserialize_missing_limit_uses_default() {
        let json = r#"{}"#;
        let opts: TimelineOptions = serde_json::from_str(json).unwrap();
        assert_eq!(opts.limit(), 20);
    }

    // ---- SearchOptions ----

    #[test]
    fn search_options_limit_clamp() {
        assert_eq!(SearchOptions::new(0).limit(), 1);
        assert_eq!(SearchOptions::new(-1).limit(), 1);
        assert_eq!(SearchOptions::new(50).limit(), 50);
        assert_eq!(SearchOptions::new(999).limit(), 100);
    }

    #[test]
    fn search_options_default() {
        let opts = SearchOptions::default();
        assert_eq!(opts.limit(), 20);
    }

    // ---- AccountPublic from Account ----

    #[test]
    fn account_public_strips_token() {
        let account = Account {
            id: "acc1".into(),
            host: "misskey.io".into(),
            token: "secret-token".into(),
            user_id: "uid1".into(),
            username: "taka".into(),
            display_name: Some("Taka".into()),
            avatar_url: Some("https://example.com/avatar.png".into()),
            software: "misskey".into(),
        };
        let public = AccountPublic::new(&account, true);
        assert_eq!(public.id, "acc1");
        assert_eq!(public.host, "misskey.io");
        assert_eq!(public.username, "taka");
        assert!(public.has_token);
        // AccountPublic has no token field
        let json = serde_json::to_value(&public).unwrap();
        assert!(json.get("token").is_none());
        assert_eq!(json.get("hasToken").unwrap(), true);
    }

    #[test]
    fn account_public_without_token() {
        let account = Account {
            id: "acc2".into(),
            host: "misskey.io".into(),
            token: "".into(),
            user_id: "uid2".into(),
            username: "user2".into(),
            display_name: None,
            avatar_url: None,
            software: "misskey".into(),
        };
        let public = AccountPublic::new(&account, false);
        assert!(!public.has_token);
        let json = serde_json::to_value(&public).unwrap();
        assert_eq!(json.get("hasToken").unwrap(), false);
    }

    // ---- Raw -> Normalized conversions ----

    fn raw_user_json() -> Value {
        json!({
            "id": "u1",
            "username": "testuser",
            "host": null,
            "name": "Test User",
            "avatarUrl": "https://example.com/avatar.png",
            "isBot": false,
            "avatarDecorations": [],
            "emojis": {},
            "instance": null
        })
    }

    fn raw_note_json() -> Value {
        json!({
            "id": "n1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "text": "Hello, world!",
            "cw": null,
            "user": raw_user_json(),
            "visibility": "public",
            "emojis": {"smile": "https://example.com/smile.png"},
            "reactionEmojis": {},
            "reactions": {":star:": 3},
            "myReaction": null,
            "renoteCount": 1,
            "repliesCount": 2,
            "files": [],
            "poll": null,
            "replyId": null,
            "renoteId": null,
            "channelId": null,
            "reactionAcceptance": null,
            "uri": null,
            "url": null,
            "updatedAt": null,
            "localOnly": false,
            "visibleUserIds": [],
            "isFavorited": false,
            "reply": null,
            "renote": null
        })
    }

    #[test]
    fn raw_note_normalize_basic() {
        let raw: RawNote = serde_json::from_value(raw_note_json()).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        assert_eq!(note.id, "n1");
        assert_eq!(note.account_id, "acc1");
        assert_eq!(note.server_host, "misskey.io");
        assert_eq!(note.text.as_deref(), Some("Hello, world!"));
        assert_eq!(note.visibility, "public");
        assert_eq!(note.user.username, "testuser");
        assert_eq!(note.renote_count, 1);
        assert_eq!(note.replies_count, 2);
        assert_eq!(*note.reactions.get(":star:").unwrap(), 3);
        assert_eq!(
            *note.emojis.get("smile").unwrap(),
            "https://example.com/smile.png"
        );
    }

    #[test]
    fn raw_note_normalize_with_poll() {
        let mut j = raw_note_json();
        j["poll"] = json!({
            "choices": [
                {"text": "Rust", "votes": 10, "isVoted": true},
                {"text": "Go", "votes": 5, "isVoted": false}
            ],
            "multiple": false,
            "expiresAt": "2025-12-31T00:00:00.000Z"
        });
        let raw: RawNote = serde_json::from_value(j).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        let poll = note.poll.unwrap();
        assert_eq!(poll.choices.len(), 2);
        assert_eq!(poll.choices[0].text, "Rust");
        assert_eq!(poll.choices[0].votes, 10);
        assert!(poll.choices[0].is_voted);
        assert!(!poll.multiple);
    }

    #[test]
    fn raw_note_normalize_with_files() {
        let mut j = raw_note_json();
        j["files"] = json!([{
            "id": "f1",
            "name": "photo.jpg",
            "type": "image/jpeg",
            "url": "https://example.com/photo.jpg",
            "thumbnailUrl": "https://example.com/photo_thumb.jpg",
            "size": 12345,
            "isSensitive": true
        }]);
        let raw: RawNote = serde_json::from_value(j).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        assert_eq!(note.files.len(), 1);
        assert_eq!(note.files[0].id, "f1");
        assert_eq!(note.files[0].file_type, "image/jpeg");
        assert!(note.files[0].is_sensitive);
        assert_eq!(note.files[0].size, 12345);
    }

    #[test]
    fn raw_note_normalize_nested_renote() {
        let mut j = raw_note_json();
        j["renoteId"] = json!("n2");
        j["renote"] = raw_note_json();
        j["renote"]["id"] = json!("n2");
        j["renote"]["text"] = json!("Original note");
        let raw: RawNote = serde_json::from_value(j).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        assert_eq!(note.renote_id.as_deref(), Some("n2"));
        let renote = note.renote.unwrap();
        assert_eq!(renote.id, "n2");
        assert_eq!(renote.text.as_deref(), Some("Original note"));
        assert_eq!(renote.account_id, "acc1");
    }

    #[test]
    fn raw_note_normalize_mode_flags() {
        let mut j = raw_note_json();
        j["isNoteInYamiMode"] = json!(true);
        j["isNoteInSuperMode"] = json!(false);
        j["unrelatedField"] = json!("ignored");
        let raw: RawNote = serde_json::from_value(j).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        assert_eq!(*note.mode_flags.get("isNoteInYamiMode").unwrap(), true);
        assert_eq!(*note.mode_flags.get("isNoteInSuperMode").unwrap(), false);
        assert!(!note.mode_flags.contains_key("unrelatedField"));
    }

    #[test]
    fn raw_note_normalize_mode_flags_non_bool_ignored() {
        let mut j = raw_note_json();
        j["isNoteInStringMode"] = json!("not a bool");
        let raw: RawNote = serde_json::from_value(j).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        assert!(!note.mode_flags.contains_key("isNoteInStringMode"));
    }

    #[test]
    fn raw_user_to_normalized() {
        let raw: RawUser = serde_json::from_value(raw_user_json()).unwrap();
        let user: NormalizedUser = raw.into();
        assert_eq!(user.id, "u1");
        assert_eq!(user.username, "testuser");
        assert!(user.host.is_none());
        assert_eq!(user.name.as_deref(), Some("Test User"));
        assert!(!user.is_bot);
    }

    #[test]
    fn raw_user_detail_normalize() {
        let j = json!({
            "id": "u1",
            "username": "testuser",
            "host": "remote.example.com",
            "name": "Test User",
            "avatarUrl": null,
            "bannerUrl": "https://example.com/banner.png",
            "description": "Hello!",
            "followersCount": 100,
            "followingCount": 50,
            "notesCount": 200,
            "isBot": true,
            "isCat": true,
            "isFollowing": true,
            "isFollowed": false,
            "createdAt": "2024-01-01T00:00:00.000Z",
            "avatarDecorations": [],
            "emojis": {},
            "roles": [{"id": "r1", "name": "Admin", "color": "#ff0000", "iconUrl": null, "description": "Administrator", "displayOrder": 1}],
            "fields": [{"name": "Website", "value": "https://example.com"}],
            "url": "https://remote.example.com/@testuser",
            "birthday": "2000-01-01",
            "location": "Tokyo",
            "onlineStatus": "online"
        });
        let raw: RawUserDetail = serde_json::from_value(j).unwrap();
        let detail = raw.normalize();
        assert_eq!(detail.id, "u1");
        assert_eq!(detail.host.as_deref(), Some("remote.example.com"));
        assert!(detail.is_bot);
        assert!(detail.is_cat);
        assert!(detail.is_following);
        assert_eq!(detail.followers_count, 100);
        assert_eq!(detail.notes_count, 200);
        assert_eq!(detail.roles.len(), 1);
        assert_eq!(detail.roles[0].name, "Admin");
        assert_eq!(detail.fields.len(), 1);
        assert_eq!(detail.fields[0].name, "Website");
        assert_eq!(detail.birthday.as_deref(), Some("2000-01-01"));
    }

    #[test]
    fn raw_notification_normalize() {
        let j = json!({
            "id": "notif1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "type": "reaction",
            "user": raw_user_json(),
            "note": raw_note_json(),
            "reaction": ":star:"
        });
        let raw: RawNotification = serde_json::from_value(j).unwrap();
        let notif = raw.normalize("acc1", "misskey.io");
        assert_eq!(notif.id, "notif1");
        assert_eq!(notif.account_id, "acc1");
        assert_eq!(notif.notification_type, "reaction");
        assert!(notif.user.is_some());
        assert!(notif.note.is_some());
        assert_eq!(notif.reaction.as_deref(), Some(":star:"));
    }

    #[test]
    fn raw_notification_normalize_without_user_or_note() {
        let j = json!({
            "id": "notif2",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "type": "followRequestAccepted",
            "user": null,
            "note": null,
            "reaction": null
        });
        let raw: RawNotification = serde_json::from_value(j).unwrap();
        let notif = raw.normalize("acc1", "misskey.io");
        assert!(notif.user.is_none());
        assert!(notif.note.is_none());
        assert!(notif.reaction.is_none());
    }

    #[test]
    fn raw_emoji_to_server_emoji() {
        let raw = RawEmoji {
            name: "blobcat".into(),
            url: "https://example.com/blobcat.png".into(),
            category: Some("blob".into()),
            aliases: vec!["cat".into(), "neko".into()],
        };
        let emoji: ServerEmoji = raw.into();
        assert_eq!(emoji.name, "blobcat");
        assert_eq!(emoji.category.as_deref(), Some("blob"));
        assert_eq!(emoji.aliases, vec!["cat", "neko"]);
    }

    #[test]
    fn raw_drive_file_to_normalized() {
        let raw = RawDriveFile {
            id: "f1".into(),
            name: "test.png".into(),
            file_type: "image/png".into(),
            url: "https://example.com/test.png".into(),
            thumbnail_url: None,
            size: 0,
            is_sensitive: false,
        };
        let file: NormalizedDriveFile = raw.into();
        assert_eq!(file.id, "f1");
        assert_eq!(file.file_type, "image/png");
        assert!(file.thumbnail_url.is_none());
    }

    #[test]
    fn raw_note_reaction_to_normalized() {
        let j = json!({
            "id": "r1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "user": raw_user_json(),
            "type": ":star:"
        });
        let raw: RawNoteReaction = serde_json::from_value(j).unwrap();
        let reaction: NormalizedNoteReaction = raw.into();
        assert_eq!(reaction.id, "r1");
        assert_eq!(reaction.reaction_type, ":star:");
        assert_eq!(reaction.user.username, "testuser");
    }

    // ---- Deserialization edge cases ----

    #[test]
    fn deserialize_note_with_minimal_fields() {
        let j = json!({
            "id": "n1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "text": null,
            "cw": null,
            "user": {"id": "u1", "username": "a"},
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
        });
        let raw: RawNote = serde_json::from_value(j).unwrap();
        assert_eq!(raw.visibility, "");
        assert!(raw.files.is_empty());
        assert!(raw.reactions.is_empty());
        assert_eq!(raw.renote_count, 0);
    }

    #[test]
    fn chat_message_with_null_reactions() {
        let j = json!({
            "id": "cm1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "fromUserId": "u1",
            "fromUser": null,
            "toUserId": null,
            "toUser": null,
            "toRoomId": null,
            "toRoom": null,
            "text": "Hello",
            "fileId": null,
            "file": null,
            "isRead": null,
            "reactions": null
        });
        let msg: ChatMessage = serde_json::from_value(j).unwrap();
        assert!(msg.reactions.is_empty());
        assert_eq!(msg.text.as_deref(), Some("Hello"));
    }

    #[test]
    fn chat_message_with_reactions() {
        let j = json!({
            "id": "cm1",
            "createdAt": "2025-01-01T00:00:00.000Z",
            "fromUserId": "u1",
            "fromUser": null,
            "toUserId": null,
            "toUser": null,
            "toRoomId": null,
            "toRoom": null,
            "text": null,
            "fileId": null,
            "file": null,
            "isRead": true,
            "reactions": [{"user": null, "reaction": ":star:"}]
        });
        let msg: ChatMessage = serde_json::from_value(j).unwrap();
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].reaction, ":star:");
    }

    #[test]
    fn normalized_note_serde_roundtrip() {
        let raw: RawNote = serde_json::from_value(raw_note_json()).unwrap();
        let note = raw.normalize("acc1", "misskey.io");
        let json = serde_json::to_string(&note).unwrap();
        let back: NormalizedNote = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, note.id);
        assert_eq!(back.account_id, note.account_id);
        assert_eq!(back.server_host, note.server_host);
    }

    #[test]
    fn create_note_params_serialize() {
        let params = CreateNoteParams {
            text: Some("test".into()),
            cw: None,
            visibility: Some("public".into()),
            local_only: Some(true),
            mode_flags: None,
            reply_id: None,
            renote_id: None,
            file_ids: None,
            poll: Some(CreateNotePoll {
                choices: vec!["A".into(), "B".into()],
                multiple: Some(false),
                expires_at: None,
            }),
            scheduled_at: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["text"], "test");
        assert_eq!(json["visibility"], "public");
        assert_eq!(json["localOnly"], true);
        assert_eq!(json["poll"]["choices"], json!(["A", "B"]));
    }

    #[test]
    fn stored_server_serde_roundtrip() {
        let server = StoredServer {
            host: "misskey.io".into(),
            software: "misskey".into(),
            version: "2024.1.0".into(),
            features_json: r#"{"miAuth":true}"#.into(),
            updated_at: 1700000000,
        };
        let json = serde_json::to_string(&server).unwrap();
        let back: StoredServer = serde_json::from_str(&json).unwrap();
        assert_eq!(back.host, "misskey.io");
        assert_eq!(back.version, "2024.1.0");
    }
}
