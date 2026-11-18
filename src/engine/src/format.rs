//! The little formatters of `ui/Format.kt`: durations as "4m", "2h 5m",
//! model slugs as display names, runtime modes as words.

use chrono::{DateTime, Utc};

/// A duration as the Java mod writes it: 40s, 5m, 2h 5m, 3d.
pub fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    } else {
        format!("{}d", seconds / 86400)
    }
}

/// How long ago `instant` was, in `duration`'s words.
pub fn since(instant: DateTime<Utc>) -> String {
    duration((Utc::now() - instant).num_seconds())
}

/// A model slug as a name: "claude-opus-4-5" -> "Claude Opus 4.5",
/// "gpt-5-1-codex" -> "GPT-5 1 Codex" (the Kotlin heuristic, kept).
pub fn model(slug: &str) -> String {
    if slug.trim().is_empty() {
        return String::new();
    }
    let lower = slug.to_lowercase();
    let tokens: Vec<&str> = lower.split('-').filter(|t| !t.is_empty()).collect();
    match tokens.first().copied() {
        Some("claude") => claude_model(&tokens[1..]),
        Some("gpt") => {
            "GPT-".to_owned()
                + &tokens[1..]
                    .iter()
                    .map(|t| capitalize(t))
                    .collect::<Vec<_>>()
                    .join(" ")
        }
        _ => tokens.iter().map(|t| capitalize(t)).collect::<Vec<_>>().join(" "),
    }
}

/// "ask-user-question" -> "Ask user question".
pub fn runtime_mode(mode: &str) -> String {
    capitalize(&mode.replace('-', " "))
}

fn claude_model(tokens: &[&str]) -> String {
    let Some(family) = tokens.iter().find(|t| t.chars().any(|c| c.is_alphabetic())) else {
        return "Claude".to_owned();
    };
    let version = tokens
        .iter()
        .filter(|t| t.chars().all(|c| c.is_ascii_digit()) && t.len() <= 2)
        .copied()
        .collect::<Vec<_>>()
        .join(".");
    [capitalize(family), version]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(5), "5s");
        assert_eq!(duration(65), "1m");
        assert_eq!(duration(3600), "1h");
        assert_eq!(duration(3900), "1h 5m");
        assert_eq!(duration(90000), "1d");
    }

    #[test]
    fn model_names() {
        // The Kotlin heuristic drops the "claude" prefix entirely.
        assert_eq!(model("claude-opus-4-5"), "Opus 4.5");
        assert_eq!(model("claude"), "Claude");
        assert_eq!(model("gpt-5-1-codex"), "GPT-5 1 Codex");
        assert_eq!(model(""), "");
    }
}
