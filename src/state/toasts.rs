use std::time::{Duration, Instant};

const MAX_VISIBLE: usize = 5;

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub created_at: Instant,
    pub duration: Duration,
    pub toast_type: ToastType,
    /// A button the toast carries, such as Undo, for as long as it shows.
    pub action: Option<ToastAction>,
    /// A second button beside it, such as Show report beside Undo import.
    pub also: Option<ToastAction>,
}

/// What a toast's button does when it is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastAction {
    /// Take back the Wardrobe change that has this number: a drop, or the removal of a garment.
    Undo(u64),
    /// Open the New project dialog, which a command that needs a project offers.
    NewProject,
    /// Take back the import that ended as the `n`th of this project, if none came after it.
    UndoImport(u64),
    /// Open the report of the import that just ended.
    ShowReport,
    /// Ask for a project to open: a project that is gone leaves the person nothing else to press.
    OpenProject,
    /// Show the folder the chosen build was written to in the file manager.
    ShowBuild,
    /// Put the keyboard in the name of the pack the Wardrobe shows, which another pack of the project may share.
    RenamePack,
    /// Save this document, which an edit made from another document's tab changed.
    SavePack(super::DocumentId),
    /// Bring the Log forward, which holds the whole of a notice too long for a toast.
    ShowLog,
}

impl ToastAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Undo(_) => tr!("Undo"),
            Self::NewProject => tr!("New project…"),
            Self::UndoImport(_) => tr!("Undo import"),
            Self::ShowReport => tr!("Show report"),
            Self::OpenProject => tr!("Open a project…"),
            Self::ShowBuild => tr!("Show build"),
            Self::RenamePack => tr!("Rename pack…"),
            Self::SavePack(_) => tr!("Save the pack"),
            Self::ShowLog => tr!("Details"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastType {
    #[default]
    Success,
    Info,
    Warning,
    Error,
}

impl Toast {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(3),
            toast_type: ToastType::Success,
            action: None,
            also: None,
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(4),
            toast_type: ToastType::Info,
            action: None,
            also: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(4),
            toast_type: ToastType::Warning,
            action: None,
            also: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(5),
            toast_type: ToastType::Error,
            action: None,
            also: None,
        }
    }

    /// The same toast with a button, which stays up long enough to be read and pressed.
    pub fn with_action(mut self, action: ToastAction) -> Self {
        self.action = Some(action);
        self.duration = Duration::from_secs(10);
        self
    }

    /// The same toast, which stays until it is clicked away: what it says blocks something until it is dealt with.
    pub fn lasting(mut self) -> Self {
        self.duration = Duration::from_secs(24 * 60 * 60);
        self
    }

    /// The same toast with a second button beside the first.
    pub fn with_also(mut self, action: ToastAction) -> Self {
        self.also = Some(action);
        self
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    pub fn remaining_fraction(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        (1.0 - elapsed / total).max(0.0)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Toasts {
    visible: Vec<Toast>,
}

impl Toasts {
    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.visible.iter()
    }

    pub fn success(&mut self, message: impl Into<String>) {
        self.push(Toast::success(message));
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.push(Toast::info(message));
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(Toast::warning(message));
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.push(Toast::error(message));
    }

    /// ⚠ **Also logged**, because a toast is gone in seconds and the Log panel
    /// is where a user goes to read what they were told.
    pub fn push(&mut self, toast: Toast) {
        match toast.toast_type {
            ToastType::Success | ToastType::Info => {
                tracing::info!(target: "openrage::toast", "{}", toast.message)
            }
            ToastType::Warning => tracing::warn!(target: "openrage::toast", "{}", toast.message),
            ToastType::Error => tracing::error!(target: "openrage::toast", "{}", toast.message),
        }
        self.visible.push(toast);
        if self.visible.len() > MAX_VISIBLE {
            let excess = self.visible.len() - MAX_VISIBLE;
            self.visible.drain(0..excess);
        }
    }

    pub fn dismiss(&mut self, index: usize) {
        if index < self.visible.len() {
            self.visible.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.visible.clear();
    }

    pub fn expire(&mut self) {
        self.visible.retain(|t| !t.is_expired());
    }

    /// A toast that is past half its time is given more, for a frame that reads no clock: the toast's fade and expiry are wall time, and a test on a loaded machine reaches its frame late.
    ///
    /// ⚠ `created_at` stays: it is part of the id of the toast's egui area, and a toast made new is a new area, whose first pass is a sizing pass that the click or the Tab of the next one lands on.
    #[cfg(test)]
    pub fn keep_fresh(&mut self) {
        for toast in self
            .visible
            .iter_mut()
            .filter(|t| t.remaining_fraction() < 0.5)
        {
            toast.duration = toast.created_at.elapsed() * 4;
        }
    }
}

impl super::AppState {
    /// A success toast: what the user asked for is done. Each other level has its own.
    pub fn show_toast(&mut self, message: impl Into<String>) {
        self.toasts.success(message);
    }

    /// A success toast with a button, such as Undo.
    pub fn show_toast_with_action(&mut self, message: impl Into<String>, action: ToastAction) {
        self.toasts
            .push(Toast::success(message).with_action(action));
    }

    pub fn show_info_toast(&mut self, message: impl Into<String>) {
        self.toasts.info(message);
    }

    /// An info toast with a button, such as New project.
    pub fn show_info_toast_with_action(&mut self, message: impl Into<String>, action: ToastAction) {
        self.toasts.push(Toast::info(message).with_action(action));
    }

    pub fn show_warning_toast(&mut self, message: impl Into<String>) {
        self.toasts.warning(message);
    }

    pub fn show_error_toast(&mut self, message: impl Into<String>) {
        let msg = message.into();
        if msg == super::retail_pending_message() {
            return;
        }
        self.toasts.error(msg);
    }

    pub fn clear_toasts(&mut self) {
        self.toasts.clear();
    }

    pub fn update_toasts(&mut self) {
        self.toasts.expire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_is_capped_and_drops_the_oldest() {
        let mut toasts = Toasts::default();
        for i in 0..20 {
            toasts.success(format!("file {i}"));
        }

        assert_eq!(toasts.visible.len(), MAX_VISIBLE);
        assert_eq!(
            toasts.iter().last().expect("newest").message,
            "file 19",
            "the newest should survive; it is the one still true"
        );
    }

    #[test]
    fn expiry_keeps_what_is_still_within_its_duration() {
        let mut toasts = Toasts::default();
        toasts.push(Toast {
            message: "stale".into(),
            created_at: Instant::now() - Duration::from_secs(60),
            duration: Duration::from_secs(3),
            toast_type: ToastType::Success,
            action: None,
            also: None,
        });
        toasts.success("fresh");

        toasts.expire();
        assert_eq!(toasts.visible.len(), 1);
        assert_eq!(toasts.iter().next().expect("fresh").message, "fresh");
    }

    #[test]
    fn a_toast_expires_once_its_duration_has_passed() {
        let toast = Toast {
            message: "Test".into(),
            created_at: Instant::now() - Duration::from_secs(4),
            duration: Duration::from_secs(3),
            toast_type: ToastType::Success,
            action: None,
            also: None,
        };
        assert!(toast.is_expired());
    }

    #[test]
    fn dismissing_a_stale_index_does_nothing() {
        let mut toasts = Toasts::default();
        toasts.success("only");
        toasts.dismiss(7);
        assert_eq!(toasts.visible.len(), 1);
    }

    #[test]
    fn the_remaining_fraction_never_goes_negative() {
        let toast = Toast {
            message: "old".into(),
            created_at: Instant::now() - Duration::from_secs(60),
            duration: Duration::from_secs(3),
            toast_type: ToastType::Success,
            action: None,
            also: None,
        };
        assert_eq!(toast.remaining_fraction(), 0.0);
    }
}
