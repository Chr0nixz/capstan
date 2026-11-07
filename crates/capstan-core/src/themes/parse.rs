use super::{
    clean_name, TerminalThemeDefinition, ThemeError, ThemeSource, MAX_IMPORT_BYTES, MAX_THEMES,
};
use quick_xml::{events::Event, Reader};
use serde_json::Value;
use uuid::Uuid;

const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
];

/// Parse one picker-selected file. The basename is only a fallback display name for iTerm2.
pub fn parse_theme_file(
    source: ThemeSource,
    bytes: &[u8],
    basename: &str,
) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ThemeError::Limit);
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match source {
        ThemeSource::WindowsTerminal => parse_windows_terminal(bytes),
        ThemeSource::Iterm2 => parse_iterm(bytes, basename).map(|theme| vec![theme]),
    }
}

pub(super) fn hex_color(input: &str) -> Result<String, ThemeError> {
    let rgb = input.strip_prefix('#').ok_or(ThemeError::Invalid)?;
    if !matches!(rgb.len(), 3 | 6 | 8) || !rgb.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ThemeError::Invalid);
    }
    if rgb.len() == 3 {
        let expanded: String = rgb.chars().flat_map(|digit| [digit, digit]).collect();
        return Ok(format!("#{}", expanded.to_ascii_lowercase()));
    }
    Ok(format!("#{}", rgb[..6].to_ascii_lowercase()))
}

fn parse_windows_terminal(bytes: &[u8]) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ThemeError::Json)?;
    let schemes = if let Some(schemes) = value.get("schemes") {
        schemes.as_array().ok_or(ThemeError::Invalid)?.as_slice()
    } else {
        std::slice::from_ref(&value)
    };
    if schemes.is_empty() {
        return Err(ThemeError::Invalid);
    }
    if schemes.len() > MAX_THEMES {
        return Err(ThemeError::Limit);
    }
    schemes
        .iter()
        .map(|value| {
            let name = clean_name(
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or(ThemeError::Invalid)?,
            )?;
            let get_color = |key: &str| {
                let alias = match key {
                    "magenta" => "purple",
                    "brightMagenta" => "brightPurple",
                    _ => key,
                };
                value
                    .get(key)
                    .or_else(|| value.get(alias))
                    .map(|value| hex_color(value.as_str().ok_or(ThemeError::Invalid)?))
                    .transpose()
            };
            let mut colors = [
                "foreground",
                "background",
                "cursorColor",
                "selectionBackground",
            ]
            .into_iter()
            .chain(ANSI_NAMES)
            .map(get_color);
            definition(ThemeSource::WindowsTerminal, name, &mut colors)
        })
        .collect()
}

fn definition(
    source: ThemeSource,
    name: String,
    colors: &mut impl Iterator<Item = Result<Option<String>, ThemeError>>,
) -> Result<TerminalThemeDefinition, ThemeError> {
    let mut next = || colors.next().ok_or(ThemeError::Invalid)?;
    let foreground = next()?.ok_or(ThemeError::Invalid)?;
    let background = next()?.ok_or(ThemeError::Invalid)?;
    let cursor = next()?.unwrap_or_else(|| foreground.clone());
    let selection_background = next()?.unwrap_or_else(|| "#404040".to_owned());
    let mut ansi = std::array::from_fn(|_| String::new());
    for color in &mut ansi {
        *color = next()?.ok_or(ThemeError::Invalid)?;
    }
    Ok(TerminalThemeDefinition {
        id: format!("custom:{}", Uuid::new_v4()),
        name,
        source,
        foreground,
        background,
        cursor,
        selection_background,
        ansi,
    })
}

fn parse_iterm(bytes: &[u8], basename: &str) -> Result<TerminalThemeDefinition, ThemeError> {
    validate_plist_xml(bytes)?;
    let value = plist::Value::from_reader_xml(bytes).map_err(|_| ThemeError::Xml)?;
    let colors = value.as_dictionary().ok_or(ThemeError::Invalid)?;
    let name = clean_name(basename)?;
    let keys = [
        "Foreground Color",
        "Background Color",
        "Cursor Color",
        "Selection Color",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain((0..16).map(|index| format!("Ansi {index} Color")));
    let mut colors = keys.map(|key| colors.get(&key).map(iterm_color).transpose());
    definition(ThemeSource::Iterm2, name, &mut colors)
}

fn iterm_color(value: &plist::Value) -> Result<String, ThemeError> {
    let components = value.as_dictionary().ok_or(ThemeError::Invalid)?;
    let channel = |key| {
        let number = match components.get(key) {
            Some(plist::Value::Real(value)) => *value,
            Some(plist::Value::Integer(value)) if value.as_unsigned() == Some(0) => 0.0,
            Some(plist::Value::Integer(value)) if value.as_unsigned() == Some(1) => 1.0,
            _ => return Err(ThemeError::Invalid),
        };
        if !(0.0..=1.0).contains(&number) {
            return Err(ThemeError::Invalid);
        }
        // The finite range above bounds the rounded sRGB channel to 0..=255.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok((number * 255.0).round() as u8)
    };
    Ok(format!(
        "#{:02x}{:02x}{:02x}",
        channel("Red Component")?,
        channel("Green Component")?,
        channel("Blue Component")?
    ))
}

fn validate_plist_xml(bytes: &[u8]) -> Result<(), ThemeError> {
    let xml = std::str::from_utf8(bytes).map_err(|_| ThemeError::Xml)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0;
    let mut root = false;
    let mut doctype = false;
    loop {
        match reader.read_event().map_err(|_| ThemeError::Xml)? {
            Event::Start(tag) => {
                if depth == 0 {
                    if root || tag.name().as_ref() != "plist" { return Err(ThemeError::Xml); }
                    root = true;
                }
                depth += 1;
                if depth > 24 { return Err(ThemeError::Limit); }
            }
            Event::End(_) if depth > 0 => depth -= 1,
            // Standard exporters include this declaration. Treat it as inert text;
            // no internal subset, custom entity or external document is admitted.
            Event::DocType(tag) if !root && !doctype && matches!(tag.as_ref(),
                "plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"" |
                "plist PUBLIC \"-//Apple Computer//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\""
            ) => doctype = true,
            Event::GeneralRef(entity) if depth > 0 && (entity.is_char_ref()
                || matches!(entity.as_ref(), "amp" | "lt" | "gt" | "apos" | "quot")) => {},
            Event::Decl(_) if !root && !doctype => {},
            Event::Comment(_) => {},
            Event::Text(text) if depth > 0 || text.as_ref().bytes().all(|byte| byte.is_ascii_whitespace()) => {},
            Event::Eof if root && depth == 0 => return Ok(()),
            _ => return Err(ThemeError::Xml),
        }
    }
}
