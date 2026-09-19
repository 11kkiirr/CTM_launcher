//! Modal forms, overlays and their actions.

use std::path::PathBuf;

use mc_core::auth::DeviceCodePrompt;
use mc_core::instance::{GcPreset, LoaderType};

/// The kind of value a form field holds.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    Number,
    Choice {
        options: Vec<String>,
        selected: usize,
    },
    Bool(bool),
}

/// A single labelled form field.
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub value: String,
    pub kind: FieldKind,
    pub hint: String,
}

impl FormField {
    pub fn editable(&self) -> bool {
        matches!(self.kind, FieldKind::Text | FieldKind::Number)
    }
}

/// What submitting a form should do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FormAction {
    CreateInstance,
    EditInstanceSettings,
    ImportModpack,
    OfflineLogin,
    SetJavaPath,
    EditLauncherSettings,
}

/// A modal multi-field form.
#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<FormField>,
    pub active: usize,
    pub action: FormAction,
}

impl Form {
    pub fn new(title: impl Into<String>, action: FormAction) -> Self {
        Self {
            title: title.into(),
            fields: Vec::new(),
            active: 0,
            action,
        }
    }

    pub fn push_text(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push(FormField {
            label: label.into(),
            value: value.into(),
            kind: FieldKind::Text,
            hint: String::new(),
        });
        self
    }

    pub fn push_number(mut self, label: impl Into<String>, value: u32) -> Self {
        self.fields.push(FormField {
            label: label.into(),
            value: value.to_string(),
            kind: FieldKind::Number,
            hint: String::new(),
        });
        self
    }

    pub fn push_choice(
        mut self,
        label: impl Into<String>,
        options: Vec<String>,
        selected: usize,
    ) -> Self {
        self.fields.push(FormField {
            label: label.into(),
            value: options.get(selected).cloned().unwrap_or_default(),
            kind: FieldKind::Choice { options, selected },
            hint: String::new(),
        });
        self
    }

    pub fn push_bool(mut self, label: impl Into<String>, value: bool) -> Self {
        self.fields.push(FormField {
            label: label.into(),
            value: if value { "yes" } else { "no" }.to_string(),
            kind: FieldKind::Bool(value),
            hint: String::new(),
        });
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        if let Some(field) = self.fields.last_mut() {
            field.hint = hint.into();
        }
        self
    }

    pub fn field(&self, label: &str) -> Option<&FormField> {
        self.fields.iter().find(|f| f.label == label)
    }

    pub fn text_value(&self, label: &str) -> Option<&str> {
        self.field(label).map(|f| f.value.as_str())
    }

    pub fn number_value(&self, label: &str) -> Option<u32> {
        self.field(label).and_then(|f| f.value.trim().parse().ok())
    }

    pub fn bool_value(&self, label: &str) -> Option<bool> {
        self.field(label).and_then(|f| match f.kind {
            FieldKind::Bool(v) => Some(v),
            _ => None,
        })
    }

    pub fn choice_value(&self, label: &str) -> Option<&str> {
        self.field(label).map(|f| f.value.as_str())
    }

    pub fn next_field(&mut self) {
        if !self.fields.is_empty() {
            self.active = (self.active + 1) % self.fields.len();
        }
    }

    pub fn prev_field(&mut self) {
        if !self.fields.is_empty() {
            self.active = (self.active + self.fields.len() - 1) % self.fields.len();
        }
    }

    /// Insert a character into the active editable field.
    pub fn input_char(&mut self, c: char) {
        if let Some(field) = self.fields.get_mut(self.active) {
            if field.editable() {
                field.value.push(c);
            }
        }
    }

    /// Remove the last character from the active editable field.
    pub fn backspace(&mut self) {
        if let Some(field) = self.fields.get_mut(self.active) {
            if field.editable() {
                field.value.pop();
            }
        }
    }

    /// Cycle a choice or toggle a boolean on the active field.
    pub fn cycle(&mut self, forward: bool) {
        if let Some(field) = self.fields.get_mut(self.active) {
            match &mut field.kind {
                FieldKind::Choice { options, selected } => {
                    if options.is_empty() {
                        return;
                    }
                    if forward {
                        *selected = (*selected + 1) % options.len();
                    } else {
                        *selected = (*selected + options.len() - 1) % options.len();
                    }
                    field.value = options[*selected].clone();
                }
                FieldKind::Bool(value) => {
                    *value = !*value;
                    field.value = if *value { "yes" } else { "no" }.to_string();
                }
                _ => {}
            }
        }
    }
}

/// What a text prompt's submitted value should do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TextAction {
    SearchModrinth,
    SearchMods,
    SearchLogs,
    ImportPath,
    JavaPath,
    CustomJvmArgs,
    CustomGameArgs,
    SkinUrl,
    RenameInstance,
    None,
}

/// What a confirmation dialog should do on accept.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ConfirmAction {
    DeleteInstance(String),
    DeleteMod(PathBuf),
    DeleteAccount(String),
    Quit,
    None,
}

/// What a version picker's chosen value applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerTarget {
    /// Create wizard: the Minecraft game version.
    WizardGame,
    /// Create wizard: the modloader version.
    WizardLoader,
    /// Versions page: change the selected build's game version.
    ChangeGameVersion,
}

/// A searchable, keyboard-driven list of versions.
#[derive(Debug, Clone)]
pub struct VersionPicker {
    pub title: String,
    pub query: String,
    pub items: Vec<String>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub target: PickerTarget,
}

impl VersionPicker {
    pub fn new(title: impl Into<String>, items: Vec<String>, target: PickerTarget) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            title: title.into(),
            query: String::new(),
            items,
            filtered,
            selected: 0,
            target,
        }
    }

    /// Recompute the filtered indices from the current query.
    pub fn refilter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| query.is_empty() || item.to_ascii_lowercase().contains(&query))
            .map(|(idx, _)| idx)
            .collect();
        self.selected = 0;
    }

    pub fn selected_value(&self) -> Option<&str> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.items.get(*idx))
            .map(String::as_str)
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let current = self.selected as i32 + delta;
        self.selected = current.clamp(0, self.filtered.len() as i32 - 1) as usize;
    }
}

/// A modal overlay currently capturing input.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Overlay {
    Text {
        title: String,
        prompt: String,
        value: String,
        action: TextAction,
    },
    Form(Form),
    Confirm {
        title: String,
        message: String,
        action: ConfirmAction,
    },
    Message {
        title: String,
        lines: Vec<String>,
    },
    DeviceCode(Box<DeviceCodePrompt>),
    Picker(VersionPicker),
    Wizard(crate::wizard::CreateWizard),
}

impl Overlay {
    pub fn text(title: impl Into<String>, prompt: impl Into<String>, action: TextAction) -> Self {
        Overlay::Text {
            title: title.into(),
            prompt: prompt.into(),
            value: String::new(),
            action,
        }
    }

    /// A text prompt with a pre-filled value.
    pub fn text_with(
        title: impl Into<String>,
        prompt: impl Into<String>,
        value: impl Into<String>,
        action: TextAction,
    ) -> Self {
        Overlay::Text {
            title: title.into(),
            prompt: prompt.into(),
            value: value.into(),
            action,
        }
    }

    pub fn confirm(
        title: impl Into<String>,
        message: impl Into<String>,
        action: ConfirmAction,
    ) -> Self {
        Overlay::Confirm {
            title: title.into(),
            message: message.into(),
            action,
        }
    }

    pub fn message(title: impl Into<String>, lines: Vec<String>) -> Self {
        Overlay::Message {
            title: title.into(),
            lines,
        }
    }
}

/// Loader options exposed in the create-instance form.
#[allow(dead_code)]
pub fn loader_options() -> Vec<String> {
    LoaderType::all()
        .iter()
        .map(|l| l.label().to_string())
        .collect()
}

/// GC options exposed in the settings form.
pub fn gc_options() -> Vec<String> {
    GcPreset::all()
        .iter()
        .map(|g| g.label().to_string())
        .collect()
}

/// Parse a loader label back into a [`LoaderType`].
#[allow(dead_code)]
pub fn parse_loader(label: &str) -> LoaderType {
    LoaderType::all()
        .iter()
        .copied()
        .find(|l| l.label().eq_ignore_ascii_case(label))
        .unwrap_or(LoaderType::Vanilla)
}

/// Parse a GC label back into a [`GcPreset`].
pub fn parse_gc(label: &str) -> GcPreset {
    GcPreset::all()
        .iter()
        .copied()
        .find(|g| g.label().eq_ignore_ascii_case(label))
        .unwrap_or(GcPreset::G1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_navigation_wraps() {
        let mut form = Form::new("t", FormAction::CreateInstance)
            .push_text("A", "a")
            .push_text("B", "b");
        assert_eq!(form.active, 0);
        form.prev_field();
        assert_eq!(form.active, 1);
        form.next_field();
        assert_eq!(form.active, 0);
    }

    #[test]
    fn form_editing_and_choice() {
        let mut form = Form::new("t", FormAction::CreateInstance)
            .push_text("Name", "")
            .push_choice("Loader", vec!["Vanilla".into(), "Fabric".into()], 0);
        form.input_char('H');
        form.input_char('i');
        assert_eq!(form.text_value("Name"), Some("Hi"));
        form.active = 1;
        form.cycle(true);
        assert_eq!(form.choice_value("Loader"), Some("Fabric"));
        form.cycle(false);
        assert_eq!(form.choice_value("Loader"), Some("Vanilla"));
    }

    #[test]
    fn bool_toggle() {
        let mut form =
            Form::new("t", FormAction::EditInstanceSettings).push_bool("Fullscreen", false);
        form.cycle(true);
        assert_eq!(form.bool_value("Fullscreen"), Some(true));
    }

    #[test]
    fn loader_round_trip() {
        for loader in LoaderType::all() {
            assert_eq!(parse_loader(loader.label()), *loader);
        }
    }
}
