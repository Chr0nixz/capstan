//! Validation shared by settings persistence and the desktop shortcut adapter.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("hotkey_invalid")]
pub struct HotkeyError;

/// Normalize a supported global shortcut into the syntax used by `global-hotkey`.
pub fn validate_hotkey(value: &str) -> Result<String, HotkeyError> {
    if value.is_empty() || value.len() > 64 || value.chars().any(char::is_control) {
        return Err(HotkeyError);
    }
    let tokens = value.split('+').map(str::trim).collect::<Vec<_>>();
    if tokens.len() < 2 || tokens.iter().any(|token| token.is_empty()) {
        return Err(HotkeyError);
    }

    let mut control = false;
    let mut alt = false;
    let mut win = false;
    let mut shift = false;
    let mut key = None;
    for token in &tokens {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" if !control => control = true,
            "alt" | "option" if !alt => alt = true,
            "win" | "windows" | "super" | "meta" | "command" | "cmd" if !win => {
                win = true;
            }
            "shift" if !shift => shift = true,
            "ctrl" | "control" | "alt" | "option" | "win" | "windows" | "super" | "meta"
            | "command" | "cmd" | "shift" => return Err(HotkeyError),
            _ if key.is_none() => key = Some(canonical_key(token).ok_or(HotkeyError)?),
            _ => return Err(HotkeyError),
        }
    }

    if !(control || alt || win) {
        return Err(HotkeyError);
    }
    let key = key.ok_or(HotkeyError)?;
    if control
        && !alt
        && !win
        && !shift
        && key.len() == 1
        && key.as_bytes()[0].is_ascii_alphabetic()
    {
        return Err(HotkeyError);
    }

    let mut parts = Vec::with_capacity(5);
    if control {
        parts.push("CTRL".to_owned());
    }
    if alt {
        parts.push("ALT".to_owned());
    }
    if shift {
        parts.push("SHIFT".to_owned());
    }
    if win {
        parts.push("SUPER".to_owned());
    }
    parts.push(key);
    Ok(parts.join("+"))
}

fn canonical_key(value: &str) -> Option<String> {
    let upper = value.to_ascii_uppercase();
    if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric()
        || matches!(
            upper.as_str(),
            "BACKQUOTE"
                | "BACKSLASH"
                | "BRACKETLEFT"
                | "BRACKETRIGHT"
                | "COMMA"
                | "EQUAL"
                | "ESCAPE"
                | "F1"
                | "F2"
                | "F3"
                | "F4"
                | "F5"
                | "F6"
                | "F7"
                | "F8"
                | "F9"
                | "F10"
                | "F11"
                | "F12"
                | "F13"
                | "F14"
                | "F15"
                | "F16"
                | "F17"
                | "F18"
                | "F19"
                | "F20"
                | "F21"
                | "F22"
                | "F23"
                | "F24"
                | "MINUS"
                | "PERIOD"
                | "QUOTE"
                | "SEMICOLON"
                | "SLASH"
                | "SPACE"
                | "TAB"
                | "ARROWUP"
                | "ARROWDOWN"
                | "ARROWLEFT"
                | "ARROWRIGHT"
                | "HOME"
                | "END"
                | "PAGEUP"
                | "PAGEDOWN"
                | "INSERT"
                | "DELETE"
                | "BACKSPACE"
        )
    {
        Some(upper)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_a_real_modifier_and_rejects_terminal_control_combinations() {
        for invalid in [
            "A",
            "Shift+A",
            "Ctrl+A",
            "Ctrl+Ctrl+F1",
            "Ctrl+A+B",
            "Ctrl+",
        ] {
            assert_eq!(validate_hotkey(invalid), Err(HotkeyError), "{invalid}");
        }
        for valid in ["Alt+Space", "Ctrl+Alt+F12", "Ctrl+Shift+C", "Win+J"] {
            assert!(validate_hotkey(valid).is_ok(), "{valid}");
        }
    }

    #[test]
    fn output_uses_the_shortcut_crate_modifier_names() {
        assert_eq!(
            validate_hotkey("win + alt + space").unwrap(),
            "ALT+SUPER+SPACE"
        );
        assert_eq!(validate_hotkey("control+shift+c").unwrap(), "CTRL+SHIFT+C");
    }

    #[test]
    fn key_and_string_boundaries_reject_partial_or_ambiguous_shortcuts() {
        for invalid in [
            "Ctrl",
            "Ctrl+Alt",
            "Ctrl+Shift",
            "Ctrl+F0",
            "Ctrl+F25",
            "Alt+Space+A",
            "Alt+Space\n",
            "Alt+\0",
            "Ctrl+Control+F1",
            "Win+Super+F1",
            "Alt++F1",
            "Alt+😀",
        ] {
            assert!(validate_hotkey(invalid).is_err(), "{invalid}");
        }
        assert!(validate_hotkey(&format!("{}Alt+F1", " ".repeat(64))).is_err());
        for valid in ["Ctrl+F1", "Ctrl+F24", "Ctrl+1", "Alt+Shift+A", "Win+A"] {
            assert!(validate_hotkey(valid).is_ok(), "{valid}");
        }
    }
}
