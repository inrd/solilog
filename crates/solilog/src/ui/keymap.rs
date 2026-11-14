//! Keymaps: the built-in defaults (`assets/keymap.json`) plus an optional
//! user file (`~/.config/solilog/keymap.json`, see [`user_keymap_path`]).
//!
//! Format (same shape as Zed's; comments and trailing commas allowed):
//!
//! ```json
//! [
//!   {
//!     "context": "IssueList",                         // optional; omitted = everywhere
//!     "bindings": {
//!       "s": "issue::SetStatus",                      // action by name
//!       "d": ["issue::SetStatus", { "status": "done" }], // action with arguments
//!       "j": null                                     // unbind a default
//!     }
//!   }
//! ]
//! ```
//!
//! User bindings are loaded after the defaults, so they win.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use gpui::{
    Action, App, Global, KeyBinding, KeyBindingContextPredicate, KeyBindingMetaIndex, NoAction,
};
use serde::Deserialize;
use serde_json::Value;

pub const DEFAULT_KEYMAP: &str = include_str!("../../assets/keymap.json");

const USER_KEYMAP_TEMPLATE: &str = r#"// Solilog keymap. Overrides the defaults; saved changes apply immediately.
// Same format as the defaults (`solilog keymap --defaults` prints them).
// Open the command palette (cmd-k) to discover action names.
[
  // {
  //   "context": "IssueList",
  //   "bindings": {
  //     "d": ["issue::SetStatus", { "status": "done" }]
  //   }
  // }
]
"#;

const META_USER: KeyBindingMetaIndex = KeyBindingMetaIndex(0);
const META_DEFAULT: KeyBindingMetaIndex = KeyBindingMetaIndex(1);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    bindings: serde_json::Map<String, Value>,
}

/// One parsed binding, before resolving the action against the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingSpec {
    pub context: Option<String>,
    pub keystrokes: String,
    /// `None` unbinds the keystrokes.
    pub action: Option<(String, Option<Value>)>,
}

pub fn parse(source: &str) -> Result<Vec<BindingSpec>> {
    let sections: Vec<Section> =
        serde_json_lenient::from_str_lenient(source).context("invalid keymap JSON")?;
    let mut specs = Vec::new();
    for section in sections {
        for (keystrokes, value) in section.bindings {
            let action = match value {
                Value::Null => None,
                Value::String(name) => Some((name, None)),
                Value::Array(items) => match items.as_slice() {
                    [Value::String(name), args] => Some((name.clone(), Some(args.clone()))),
                    _ => anyhow::bail!(
                        "binding {keystrokes:?}: expected [\"action::Name\", {{ args }}]"
                    ),
                },
                other => anyhow::bail!("binding {keystrokes:?}: unexpected value {other}"),
            };
            specs.push(BindingSpec {
                context: section.context.clone(),
                keystrokes,
                action,
            });
        }
    }
    Ok(specs)
}

type BuildAction<'a> = &'a dyn Fn(&str, Option<Value>) -> Result<Box<dyn Action>>;
type Resolved = (Box<dyn Action>, Option<Rc<KeyBindingContextPredicate>>);

fn resolve(spec: &BindingSpec, build: BuildAction) -> Result<Resolved> {
    let action = match &spec.action {
        None => Box::new(NoAction) as Box<dyn Action>,
        Some((name, args)) => build(name, args.clone())?,
    };
    let predicate = spec
        .context
        .as_deref()
        .map(KeyBindingContextPredicate::parse)
        .transpose()
        .with_context(|| format!("invalid context {:?}", spec.context))?
        .map(Rc::new);
    Ok((action, predicate))
}

fn load_bindings(
    source: &str,
    meta: KeyBindingMetaIndex,
    cx: &App,
) -> (Vec<KeyBinding>, Vec<anyhow::Error>) {
    let specs = match parse(source) {
        Ok(specs) => specs,
        Err(err) => return (Vec::new(), vec![err]),
    };
    let build = |name: &str, args| Ok(cx.build_action(name, args)?);
    let mut bindings = Vec::new();
    let mut errors = Vec::new();
    for spec in &specs {
        let result = resolve(spec, &build).and_then(|(action, predicate)| {
            KeyBinding::load(
                &spec.keystrokes,
                action,
                predicate,
                false,
                None,
                cx.keyboard_mapper().as_ref(),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))
        });
        match result {
            Ok(mut binding) => {
                binding.set_meta(meta);
                bindings.push(binding);
            }
            Err(err) => errors.push(err.context(format!("binding {:?}", spec.keystrokes))),
        }
    }
    (bindings, errors)
}

/// Where the user keymap lives: `$SOLILOG_CONFIG_DIR/keymap.json`, else
/// `$XDG_CONFIG_HOME/solilog/keymap.json`, else `~/.config/solilog/keymap.json`.
pub fn user_keymap_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("keymap.json"))
}

fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SOLILOG_CONFIG_DIR") {
        return Some(dir.into());
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("solilog"));
    }
    dirs::home_dir().map(|home| home.join(".config").join("solilog"))
}

/// Creates the user keymap from a commented template if it doesn't exist.
pub fn ensure_user_keymap() -> Result<PathBuf> {
    let path = user_keymap_path().context("no home directory")?;
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, USER_KEYMAP_TEMPLATE)?;
    }
    Ok(path)
}

/// Tracks the user keymap's mtime so edits can be picked up automatically.
#[derive(Default)]
struct KeymapState {
    user_mtime: Option<SystemTime>,
}

impl Global for KeymapState {}

fn user_keymap_mtime() -> Option<SystemTime> {
    std::fs::metadata(user_keymap_path()?).ok()?.modified().ok()
}

/// (Re)loads default + user bindings, replacing all current ones.
/// Invalid user bindings are skipped and reported on stderr.
pub fn load(cx: &mut App) {
    let (mut bindings, errors) = load_bindings(DEFAULT_KEYMAP, META_DEFAULT, cx);
    for err in errors {
        // A broken default keymap is a bug; the tests should catch it.
        eprintln!("default keymap: {err:#}");
    }

    if let Some(path) = user_keymap_path()
        && path.exists()
    {
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                let (user, errors) = load_bindings(&source, META_USER, cx);
                bindings.extend(user);
                for err in errors {
                    eprintln!("{}: {err:#}", path.display());
                }
            }
            Err(err) => eprintln!("reading {}: {err}", path.display()),
        }
    }

    cx.clear_key_bindings();
    cx.bind_keys(bindings);
    cx.set_global(KeymapState {
        user_mtime: user_keymap_mtime(),
    });
}

/// Reloads the keymap if the user file changed since the last load.
pub fn reload_if_changed(cx: &mut App) {
    let mtime = user_keymap_mtime();
    if cx.default_global::<KeymapState>().user_mtime != mtime {
        load(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds actions without a running app, straight from the registry.
    fn build(name: &str, args: Option<Value>) -> Result<Box<dyn Action>> {
        // Force the linker to keep our action registrations.
        let _ = crate::ui::actions::workspace::Quit;
        let data = gpui::generate_list_of_all_registered_actions()
            .find(|a| a.name == name)
            .with_context(|| format!("unknown action {name:?}"))?;
        (data.build)(args.unwrap_or_else(|| serde_json::json!({})))
    }

    #[test]
    fn default_keymap_is_valid() {
        let specs = parse(DEFAULT_KEYMAP).unwrap();
        assert!(!specs.is_empty());
        for spec in &specs {
            resolve(spec, &build).unwrap_or_else(|e| panic!("{spec:?}: {e:#}"));
            for ks in spec.keystrokes.split_whitespace() {
                gpui::Keystroke::parse(ks).unwrap_or_else(|e| panic!("{ks:?}: {e:?}"));
            }
        }
    }

    #[test]
    fn user_template_is_valid() {
        assert!(parse(USER_KEYMAP_TEMPLATE).unwrap().is_empty());
    }

    #[test]
    fn parse_forms() {
        let specs = parse(
            r#"[{ "context": "X", "bindings": {
                "a": "issue::New",
                "b": ["issue::SetStatus", { "status": "done" }],
                "c": null, // trailing comma and comments are fine
            }}]"#,
        )
        .unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[2].action, None);
        let (action, _) = resolve(&specs[1], &build).unwrap();
        assert_eq!(action.name(), "issue::SetStatus");
        assert!(
            resolve(
                &BindingSpec {
                    context: None,
                    keystrokes: "x".into(),
                    action: Some((
                        "issue::SetStatus".into(),
                        Some(serde_json::json!({"status": "nope"}))
                    )),
                },
                &build
            )
            .is_err()
        );
    }
}
