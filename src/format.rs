use crate::models::{NormalizedNote, NormalizedNotification, NormalizedUser, ServerEmoji};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Default,
    Json,
    Ids,
    Compact,
    Jsonl,
}

impl OutputFormat {
    pub fn from_cli(cli: &crate::cli::Cli) -> Self {
        if cli.json {
            Self::Json
        } else if cli.ids {
            Self::Ids
        } else if cli.compact {
            Self::Compact
        } else if cli.jsonl {
            Self::Jsonl
        } else {
            Self::Default
        }
    }
}

// --- Action result helper ---

/// Print a simple action result (used by delete, react, follow, favorite, etc.)
pub fn print_action(fmt: OutputFormat, json_obj: &str, id: &str, default_msg: &str) {
    match fmt {
        OutputFormat::Json | OutputFormat::Jsonl => println!("{json_obj}"),
        OutputFormat::Ids => println!("{id}"),
        _ => println!("{default_msg}"),
    }
}

// --- Notes ---

pub fn print_notes(notes: &[NormalizedNote], fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(notes).unwrap());
        }
        OutputFormat::Jsonl => {
            for note in notes {
                println!("{}", serde_json::to_string(note).unwrap());
            }
        }
        OutputFormat::Ids => {
            for note in notes {
                println!("{}", note.id);
            }
        }
        OutputFormat::Compact => {
            for note in notes {
                print_note_compact(note);
            }
        }
        OutputFormat::Default => {
            for note in notes {
                print_note_detail(note);
            }
        }
    }
}

pub fn print_note_compact(note: &NormalizedNote) {
    let user = format_user(&note.user);
    let time = &note.created_at[..16].replace('T', " ");
    let text = compact_note_text(note);
    println!("{}\t{}\t{}\t{}", note.id, user, time, text);
}

pub fn print_note_detail(note: &NormalizedNote) {
    let user = format_user(&note.user);
    let time = &note.created_at[..16].replace('T', " ");
    println!("{user}  {time}  id:{}", note.id);
    if let Some(ref cw) = note.cw {
        println!("[CW: {cw}]");
    }
    if let Some(ref text) = note.text {
        println!("{text}");
    }
    if let Some(ref renote) = note.renote {
        let ru = format_user(&renote.user);
        println!("  RN {ru}: {}", renote.text.as_deref().unwrap_or(""));
    }
    let reactions: i64 = note.reactions.values().sum();
    if reactions > 0 || note.renote_count > 0 || note.replies_count > 0 {
        println!(
            "  reactions:{} renotes:{} replies:{}",
            reactions, note.renote_count, note.replies_count
        );
    }
    println!();
}

// --- Notifications ---

pub fn print_notifications(notifications: &[NormalizedNotification], fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(notifications).unwrap());
        }
        OutputFormat::Jsonl => {
            for n in notifications {
                println!("{}", serde_json::to_string(n).unwrap());
            }
        }
        OutputFormat::Ids => {
            for n in notifications {
                if let Some(ref note) = n.note {
                    println!("{}", note.id);
                } else {
                    println!("{}", n.id);
                }
            }
        }
        OutputFormat::Compact => {
            for n in notifications {
                print_notification_compact(n);
            }
        }
        OutputFormat::Default => {
            for n in notifications {
                print_notification(n);
            }
        }
    }
}

fn print_notification_compact(n: &NormalizedNotification) {
    let user = n.user.as_ref().map(format_user).unwrap_or_default();
    let note_id = n.note.as_ref().map(|n| n.id.as_str()).unwrap_or("-");
    let reaction = n.reaction.as_deref().unwrap_or("");
    let preview = n
        .note
        .as_ref()
        .and_then(|n| n.text.as_deref())
        .map(oneline)
        .unwrap_or_default();
    println!(
        "{}\t{}\t{}\t{}\t{}",
        note_id, n.notification_type, user, reaction, preview
    );
}

fn print_notification(n: &NormalizedNotification) {
    let user = n.user.as_ref().map(format_user).unwrap_or_default();
    let note_preview = n
        .note
        .as_ref()
        .and_then(|n| n.text.as_deref())
        .unwrap_or("");
    let preview = if note_preview.len() > 50 {
        format!("{}...", &note_preview[..50])
    } else {
        note_preview.to_string()
    };

    match n.notification_type.as_str() {
        "reaction" => {
            let reaction = n.reaction.as_deref().unwrap_or("?");
            println!("reaction  {user}  {reaction}  \"{preview}\"");
        }
        "follow" => println!("follow    {user}"),
        "reply" => println!("reply     {user}  \"{preview}\""),
        "renote" => println!("renote    {user}  \"{preview}\""),
        "mention" => println!("mention   {user}  \"{preview}\""),
        "quote" => println!("quote     {user}  \"{preview}\""),
        other => println!("{:<10}{user}  \"{preview}\"", other),
    }
}

// --- User ---

pub fn print_user_detail(u: &crate::models::NormalizedUserDetail) {
    let host_str = u.host.as_deref().unwrap_or("(local)");
    let name = u.name.as_deref().unwrap_or(&u.username);
    println!("{} (@{}@{})", name, u.username, host_str);
    println!("  ID: {}", u.id);
    if let Some(ref desc) = u.description {
        println!("  Bio: {}", desc);
    }
    println!(
        "  Notes: {}  Following: {}  Followers: {}",
        u.notes_count, u.following_count, u.followers_count
    );
    if u.is_bot {
        print!("  [Bot]");
    }
    if u.is_cat {
        print!("  [Cat]");
    }
    if u.is_following {
        print!("  [Following]");
    }
    if u.is_followed {
        print!("  [Followed by]");
    }
    if u.is_bot || u.is_cat || u.is_following || u.is_followed {
        println!();
    }
    println!();
}

// --- Emojis ---

pub fn print_emojis(emojis: &[ServerEmoji], fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(emojis).unwrap());
        }
        OutputFormat::Jsonl => {
            for e in emojis {
                println!("{}", serde_json::to_string(e).unwrap());
            }
        }
        OutputFormat::Ids => {
            for e in emojis {
                println!(":{}:", e.name);
            }
        }
        OutputFormat::Compact => {
            for e in emojis {
                let cat = e.category.as_deref().unwrap_or("");
                println!(":{}:\t{}\t{}", e.name, cat, e.aliases.join(","));
            }
        }
        OutputFormat::Default => {
            let mut by_category: std::collections::BTreeMap<&str, Vec<&ServerEmoji>> =
                std::collections::BTreeMap::new();
            for emoji in emojis {
                let cat = emoji.category.as_deref().unwrap_or("(uncategorized)");
                by_category.entry(cat).or_default().push(emoji);
            }
            for (category, list) in &by_category {
                println!("[{}]", category);
                for emoji in list {
                    let aliases = if emoji.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", emoji.aliases.join(", "))
                    };
                    println!("  :{}: {}", emoji.name, aliases);
                }
            }
            println!("\nTotal: {} emojis", emojis.len());
        }
    }
}

// --- Utilities ---

pub fn format_user(user: &NormalizedUser) -> String {
    match &user.host {
        Some(host) => format!("@{}@{}", user.username, host),
        None => format!("@{}", user.username),
    }
}

pub fn oneline(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_note_text(note: &NormalizedNote) -> String {
    let mut parts = Vec::new();
    if let Some(ref cw) = note.cw {
        parts.push(format!("[CW: {}]", oneline(cw)));
    }
    if let Some(ref text) = note.text {
        parts.push(oneline(text));
    } else if let Some(ref renote) = note.renote {
        let ru = format_user(&renote.user);
        let rt = renote.text.as_deref().unwrap_or("");
        parts.push(format!("[RN {}] {}", ru, oneline(rt)));
    }
    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join(" ")
    }
}
