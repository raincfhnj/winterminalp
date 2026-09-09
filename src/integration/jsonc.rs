use std::collections::{HashMap, HashSet};

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstNode, CstRootNode};
use serde_json::Value;

use crate::{AppError, AppResult};

use super::manifest::ManagedKeybindingManifest;
use super::{ConflictKind, IntegrationConflict};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone)]
pub(crate) struct DesiredKeybinding {
    pub canonical_id: String,
    pub canonical_chord: String,
    pub definition: Value,
}

#[derive(Debug)]
pub(crate) struct SettingsEdit {
    pub existing_binding_count: usize,
    pub matching_binding_count: usize,
    pub additions: Vec<ManagedKeybindingManifest>,
    pub conflicts: Vec<IntegrationConflict>,
    pub replacement: Option<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct RemovalEdit {
    pub removed_binding_count: usize,
    pub preserved_binding_count: usize,
    pub retained: Vec<ManagedKeybindingManifest>,
    pub replacement: Option<Vec<u8>>,
}

#[derive(Debug)]
struct ParsedDocument {
    root: CstRootNode,
    had_bom: bool,
}

#[derive(Debug, Clone)]
struct ExistingKeybinding {
    node: CstNode,
    canonical_id: Option<String>,
    canonical_chord: String,
    has_only_id_and_keys: bool,
}

pub(crate) fn desired_keybinding(definition: Value) -> AppResult<DesiredKeybinding> {
    let (canonical_id, canonical_chords, has_only_id_and_keys) =
        parse_binding_definition(&definition).map_err(AppError::InvalidConfiguration)?;
    let Some(canonical_id) = canonical_id else {
        return Err(AppError::InvalidConfiguration(
            "managed keybinding definitions must include an id".to_owned(),
        ));
    };
    if !has_only_id_and_keys {
        return Err(AppError::InvalidConfiguration(
            "managed keybinding definitions may contain only id and a single string keys value"
                .to_owned(),
        ));
    }
    let [canonical_chord] = canonical_chords.as_slice() else {
        return Err(AppError::InvalidConfiguration(
            "managed keybinding definitions must contain exactly one chord".to_owned(),
        ));
    };
    Ok(DesiredKeybinding {
        canonical_id,
        canonical_chord: canonical_chord.clone(),
        definition,
    })
}

pub(crate) fn validate_desired_bindings(bindings: &[DesiredKeybinding]) -> AppResult<()> {
    let mut ids = HashSet::new();
    let mut chords = HashSet::new();
    for binding in bindings {
        if !ids.insert(binding.canonical_id.clone()) {
            return Err(AppError::InvalidConfiguration(format!(
                "managed action id {} is duplicated",
                binding.canonical_id
            )));
        }
        if !chords.insert(binding.canonical_chord.clone()) {
            return Err(AppError::InvalidConfiguration(format!(
                "managed key chord {} is duplicated",
                binding.canonical_chord
            )));
        }
    }
    Ok(())
}

pub(crate) fn merge_keybindings(
    raw: &[u8],
    desired: &[DesiredKeybinding],
) -> AppResult<SettingsEdit> {
    let document = parse_document(raw)?;
    let root_object = document.root.object_value().ok_or_else(|| {
        AppError::InvalidConfiguration("settings root must be an object".to_owned())
    })?;
    let duplicate_keybinding_properties = root_object
        .properties()
        .iter()
        .filter(|property| {
            property
                .name()
                .and_then(|name| name.decoded_value().ok())
                .as_deref()
                == Some("keybindings")
        })
        .count();
    if duplicate_keybinding_properties > 1 {
        return Ok(SettingsEdit {
            existing_binding_count: 0,
            matching_binding_count: 0,
            additions: Vec::new(),
            conflicts: vec![conflict(
                ConflictKind::InvalidSettingsShape,
                None,
                None,
                "settings contains duplicate root keybindings properties",
            )],
            replacement: None,
        });
    }

    let elements = match root_object.get("keybindings") {
        Some(_) => match root_object.array_value("keybindings") {
            Some(array) => array.elements(),
            None => {
                return Ok(SettingsEdit {
                    existing_binding_count: 0,
                    matching_binding_count: 0,
                    additions: Vec::new(),
                    conflicts: vec![conflict(
                        ConflictKind::InvalidSettingsShape,
                        None,
                        None,
                        "root keybindings must be an array",
                    )],
                    replacement: None,
                });
            }
        },
        None => Vec::new(),
    };
    let existing_binding_count = elements.len();
    let (existing, mut conflicts) = parse_existing_bindings(elements);

    let mut additions = Vec::new();
    let mut present = Vec::new();
    for managed in desired {
        let same_id: Vec<_> = existing
            .iter()
            .filter(|binding| {
                binding.canonical_id.as_deref() == Some(managed.canonical_id.as_str())
            })
            .collect();
        let same_chord: Vec<_> = existing
            .iter()
            .filter(|binding| binding.canonical_chord == managed.canonical_chord)
            .collect();
        let equivalent = existing.iter().any(|binding| {
            binding.canonical_id.as_deref() == Some(managed.canonical_id.as_str())
                && binding.canonical_chord == managed.canonical_chord
                && binding.has_only_id_and_keys
        });
        if equivalent {
            present.push(to_manifest_binding(managed));
            continue;
        }
        if !same_id.is_empty() {
            conflicts.push(conflict(
                ConflictKind::SameIdDifferentBinding,
                Some(managed.canonical_id.clone()),
                Some(managed.canonical_chord.clone()),
                format!(
                    "managed id {} already exists with a different chord or definition",
                    managed.canonical_id
                ),
            ));
            continue;
        }
        if !same_chord.is_empty() {
            let occupant = same_chord[0]
                .canonical_id
                .as_deref()
                .unwrap_or("an unmanaged keybinding");
            conflicts.push(conflict(
                ConflictKind::SameChordDifferentBinding,
                Some(managed.canonical_id.clone()),
                Some(managed.canonical_chord.clone()),
                format!(
                    "managed chord {} is already assigned to {}",
                    managed.canonical_chord, occupant
                ),
            ));
            continue;
        }
        additions.push(to_manifest_binding(managed));
    }
    deduplicate_conflicts(&mut conflicts);
    if !conflicts.is_empty() || additions.is_empty() {
        return Ok(SettingsEdit {
            existing_binding_count,
            matching_binding_count: present.len(),
            additions,
            conflicts,
            replacement: None,
        });
    }

    let array = root_object
        .array_value_or_create("keybindings")
        .ok_or_else(|| AppError::InvalidConfiguration("keybindings is not an array".to_owned()))?;
    for addition in &additions {
        array.append(value_to_cst(&addition.definition)?);
    }
    let replacement = serialize_document(&document);
    Ok(SettingsEdit {
        existing_binding_count,
        matching_binding_count: present.len(),
        additions,
        conflicts,
        replacement: Some(replacement),
    })
}

pub(crate) fn remove_managed_keybindings(
    raw: &[u8],
    managed: &[ManagedKeybindingManifest],
) -> AppResult<RemovalEdit> {
    let document = parse_document(raw)?;
    let root_object = document.root.object_value().ok_or_else(|| {
        AppError::InvalidConfiguration("settings root must be an object".to_owned())
    })?;
    let Some(array) = root_object.array_value("keybindings") else {
        if root_object.get("keybindings").is_some() {
            return Err(AppError::InvalidConfiguration(
                "root keybindings must be an array".to_owned(),
            ));
        }
        return Ok(RemovalEdit {
            removed_binding_count: 0,
            preserved_binding_count: 0,
            retained: Vec::new(),
            replacement: None,
        });
    };
    let (existing, parse_conflicts) = parse_existing_bindings(array.elements());
    if !parse_conflicts.is_empty() {
        return Err(AppError::SettingsConflict(
            "settings contains malformed keybindings; managed entries were retained".to_owned(),
        ));
    }

    let mut consumed = HashSet::new();
    let mut nodes_to_remove = Vec::new();
    let mut retained = Vec::new();
    for record in managed {
        let expected = desired_keybinding(record.definition.clone())?;
        let exact = existing.iter().enumerate().find(|(index, binding)| {
            !consumed.contains(index)
                && binding.canonical_id.as_deref() == Some(expected.canonical_id.as_str())
                && binding.canonical_chord == expected.canonical_chord
                && binding.has_only_id_and_keys
        });
        if let Some((index, binding)) = exact {
            consumed.insert(index);
            nodes_to_remove.push(binding.node.clone());
            continue;
        }

        let was_modified = existing.iter().any(|binding| {
            binding.canonical_id.as_deref() == Some(record.canonical_id.as_str())
                || binding.canonical_chord == record.canonical_chord
        });
        if was_modified {
            retained.push(record.clone());
        }
    }

    let removed_binding_count = nodes_to_remove.len();
    for node in nodes_to_remove {
        node.remove();
    }
    let replacement = (removed_binding_count > 0).then(|| serialize_document(&document));
    Ok(RemovalEdit {
        removed_binding_count,
        preserved_binding_count: retained.len(),
        retained,
        replacement,
    })
}

fn parse_document(raw: &[u8]) -> AppResult<ParsedDocument> {
    let (had_bom, source) = match raw.strip_prefix(UTF8_BOM) {
        Some(source) => (true, source),
        None => (false, raw),
    };
    let source = std::str::from_utf8(source).map_err(|error| {
        AppError::InvalidConfiguration(format!("settings is not valid UTF-8: {error}"))
    })?;
    let options = ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: true,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    let root = CstRootNode::parse(source, &options).map_err(|error| {
        AppError::InvalidConfiguration(format!("settings JSONC could not be parsed: {error}"))
    })?;
    Ok(ParsedDocument { root, had_bom })
}

fn serialize_document(document: &ParsedDocument) -> Vec<u8> {
    let serialized = document.root.to_string();
    let mut bytes = Vec::with_capacity(serialized.len() + usize::from(document.had_bom) * 3);
    if document.had_bom {
        bytes.extend_from_slice(UTF8_BOM);
    }
    bytes.extend_from_slice(serialized.as_bytes());
    bytes
}

fn parse_existing_bindings(
    elements: Vec<CstNode>,
) -> (Vec<ExistingKeybinding>, Vec<IntegrationConflict>) {
    let mut parsed = Vec::new();
    let mut conflicts = Vec::new();
    for node in elements {
        let Some(value) = node.to_serde_value() else {
            conflicts.push(conflict(
                ConflictKind::MalformedKeybinding,
                None,
                None,
                "keybindings contains a value that cannot be represented as JSON",
            ));
            continue;
        };
        match parse_binding_definition(&value) {
            Ok((canonical_id, canonical_chords, has_only_id_and_keys)) => {
                for canonical_chord in canonical_chords {
                    parsed.push(ExistingKeybinding {
                        node: node.clone(),
                        canonical_id: canonical_id.clone(),
                        canonical_chord,
                        has_only_id_and_keys,
                    });
                }
            }
            Err(message) => conflicts.push(conflict(
                ConflictKind::MalformedKeybinding,
                None,
                None,
                message,
            )),
        }
    }
    (parsed, conflicts)
}

/// Parses one root `keybindings` entry.
///
/// Returns the optional managed `id`, every chord declared by `keys` (a string
/// or an array of strings), and whether the entry is a bare `{id, keys}` object
/// with a single string chord that WinTerminal++ is allowed to own or remove.
fn parse_binding_definition(value: &Value) -> Result<(Option<String>, Vec<String>, bool), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "each root keybindings entry must be an object".to_owned())?;
    let canonical_id = match object.get("id") {
        Some(Value::String(id)) => Some(normalize_id(id)?),
        Some(_) => return Err("keybinding id must be a string".to_owned()),
        None => None,
    };
    let keys = object.get("keys").ok_or_else(|| match &canonical_id {
        Some(id) => format!("keybinding {id} is missing keys"),
        None => "keybinding entry is missing keys".to_owned(),
    })?;
    let (canonical_chords, keys_is_string) = match keys {
        Value::String(keys) => (vec![normalize_chord(keys)?], true),
        Value::Array(keys) => {
            let mut chords = Vec::with_capacity(keys.len());
            for entry in keys {
                let chord = entry
                    .as_str()
                    .ok_or_else(|| "keybinding keys array must contain only strings".to_owned())?;
                chords.push(normalize_chord(chord)?);
            }
            if chords.is_empty() {
                return Err("keybinding keys array cannot be empty".to_owned());
            }
            (chords, false)
        }
        _ => return Err("keybinding has an invalid keys value".to_owned()),
    };
    let has_only_id_and_keys = canonical_id.is_some() && object.len() == 2 && keys_is_string;
    Ok((canonical_id, canonical_chords, has_only_id_and_keys))
}

fn normalize_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("keybinding id cannot be empty".to_owned());
    }
    Ok(id.to_ascii_lowercase())
}

fn normalize_chord(chord: &str) -> Result<String, String> {
    let chord = chord.trim().to_ascii_lowercase();
    if chord.is_empty() {
        return Err("keybinding chord cannot be empty".to_owned());
    }
    let aliases: HashMap<&str, &str> = HashMap::from([
        ("control", "ctrl"),
        ("windows", "win"),
        ("escape", "esc"),
        ("return", "enter"),
        ("pageup", "pgup"),
        ("pagedown", "pgdn"),
    ]);
    let mut modifiers = HashSet::new();
    let mut key = None;
    for part in chord.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(format!("invalid keybinding chord {chord}"));
        }
        let normalized = aliases.get(part).copied().unwrap_or(part);
        if matches!(normalized, "ctrl" | "shift" | "alt" | "win") {
            if !modifiers.insert(normalized) {
                return Err(format!("duplicate modifier in keybinding chord {chord}"));
            }
        } else if key.replace(normalized).is_some() {
            return Err(format!("keybinding chord {chord} contains multiple keys"));
        }
    }
    let key = key.ok_or_else(|| format!("keybinding chord {chord} has no key"))?;
    let mut canonical = Vec::new();
    for modifier in ["ctrl", "shift", "alt", "win"] {
        if modifiers.contains(modifier) {
            canonical.push(modifier);
        }
    }
    canonical.push(key);
    Ok(canonical.join("+"))
}

fn to_manifest_binding(binding: &DesiredKeybinding) -> ManagedKeybindingManifest {
    ManagedKeybindingManifest {
        canonical_id: binding.canonical_id.clone(),
        canonical_chord: binding.canonical_chord.clone(),
        definition: binding.definition.clone(),
    }
}

fn value_to_cst(value: &Value) -> AppResult<CstInputValue> {
    match value {
        Value::Null => Ok(CstInputValue::Null),
        Value::Bool(value) => Ok(CstInputValue::Bool(*value)),
        Value::Number(value) => Ok(CstInputValue::Number(value.to_string())),
        Value::String(value) => Ok(CstInputValue::String(value.clone())),
        Value::Array(values) => values
            .iter()
            .map(value_to_cst)
            .collect::<AppResult<Vec<_>>>()
            .map(CstInputValue::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), value_to_cst(value)?)))
            .collect::<AppResult<Vec<_>>>()
            .map(CstInputValue::Object),
    }
}

fn conflict(
    kind: ConflictKind,
    action_id: Option<String>,
    keys: Option<String>,
    message: impl Into<String>,
) -> IntegrationConflict {
    IntegrationConflict {
        kind,
        action_id,
        keys,
        message: message.into(),
    }
}

fn deduplicate_conflicts(conflicts: &mut Vec<IntegrationConflict>) {
    let mut seen = HashSet::new();
    conflicts.retain(|conflict| {
        seen.insert((
            conflict.kind,
            conflict.action_id.clone(),
            conflict.keys.clone(),
            conflict.message.clone(),
        ))
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn desired(id: &str, keys: &str) -> DesiredKeybinding {
        desired_keybinding(json!({ "id": id, "keys": keys }))
            .expect("managed fixture should be valid")
    }

    #[test]
    fn merge_is_lossless_around_the_edited_array_and_idempotent() {
        let source = b"\xef\xbb\xbf{\r\n  // keep this comment\r\n  \"profiles\": [],\r\n}\r\n";
        let bindings = [desired("WinTerminalPP.SplitLeft", "ctrl+f13")];

        let first = merge_keybindings(source, &bindings).expect("merge should succeed");
        assert!(first.conflicts.is_empty());
        let replacement = first.replacement.expect("binding should be appended");
        assert!(replacement.starts_with(UTF8_BOM));
        let text = std::str::from_utf8(&replacement[3..]).expect("result should be UTF-8");
        assert!(text.contains("// keep this comment\r\n"));
        assert!(text.contains("\"profiles\": []"));

        let second = merge_keybindings(&replacement, &bindings).expect("second merge should work");
        assert!(second.conflicts.is_empty());
        assert!(second.replacement.is_none());
        assert_eq!(second.matching_binding_count, 1);
    }

    #[test]
    fn blocks_same_id_with_a_different_chord() {
        let source = br#"{"keybindings":[{"id":"WinTerminalPP.SplitLeft","keys":"ctrl+f14"}]}"#;
        let result = merge_keybindings(source, &[desired("WinTerminalPP.SplitLeft", "ctrl+f13")])
            .expect("analysis should complete");

        assert!(result.replacement.is_none());
        assert!(
            result
                .conflicts
                .iter()
                .any(|conflict| { conflict.kind == ConflictKind::SameIdDifferentBinding })
        );
    }

    #[test]
    fn blocks_same_chord_with_a_different_id_after_normalization() {
        let source = br#"{"keybindings":[{"id":"User.Action","keys":"SHIFT + CTRL + F13"}]}"#;
        let result = merge_keybindings(
            source,
            &[desired("WinTerminalPP.SplitLeft", "ctrl+shift+f13")],
        )
        .expect("analysis should complete");

        assert!(result.replacement.is_none());
        assert!(
            result
                .conflicts
                .iter()
                .any(|conflict| { conflict.kind == ConflictKind::SameChordDifferentBinding })
        );
    }

    #[test]
    fn uninstall_removes_only_semantically_unchanged_manifest_entries() {
        let source = br#"{
  "keybindings": [
    { "id": "WinTerminalPP.SplitLeft", "keys": "ctrl+f13" },
    { "id": "WinTerminalPP.SplitRight", "keys": "ctrl+f24", "userNote": true },
    { "id": "User.Action", "keys": "ctrl+x" }
  ]
}"#;
        let records = [
            to_manifest_binding(&desired("WinTerminalPP.SplitLeft", "ctrl+f13")),
            to_manifest_binding(&desired("WinTerminalPP.SplitRight", "ctrl+f14")),
        ];

        let edit = remove_managed_keybindings(source, &records).expect("removal should succeed");
        let replacement = edit.replacement.expect("one binding should be removed");
        let text = std::str::from_utf8(&replacement).expect("result should be UTF-8");

        assert_eq!(edit.removed_binding_count, 1);
        assert_eq!(edit.preserved_binding_count, 1);
        assert!(!text.contains("SplitLeft"));
        assert!(text.contains("SplitRight"));
        assert!(text.contains("User.Action"));
    }

    #[test]
    fn command_keybindings_without_id_are_not_malformed() {
        let source = br#"{"keybindings":[{"command":"newTab","keys":"ctrl+shift+t"}]}"#;
        let result = merge_keybindings(
            source,
            &[desired("WinTerminalPP.SplitLeft", "ctrl+alt+shift+f13")],
        )
        .expect("analysis should complete");

        assert!(result.conflicts.is_empty());
        assert!(result.replacement.is_some());
    }

    #[test]
    fn multi_chord_entries_are_not_malformed_but_still_reserve_their_chords() {
        let source =
            br#"{"keybindings":[{"id":"User.Multi","keys":["ctrl+alt+shift+f13","ctrl+x"]}]}"#;
        let result = merge_keybindings(
            source,
            &[desired("WinTerminalPP.SplitLeft", "ctrl+alt+shift+f13")],
        )
        .expect("analysis should complete");

        assert!(
            result
                .conflicts
                .iter()
                .any(|conflict| conflict.kind == ConflictKind::SameChordDifferentBinding)
        );
        assert!(
            !result
                .conflicts
                .iter()
                .any(|conflict| conflict.kind == ConflictKind::MalformedKeybinding)
        );
    }
}
