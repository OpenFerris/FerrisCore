use leptos::prelude::*;

pub type ToastId = u64;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToastLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToastPosition {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/* ========================================================== */
/*                       🧬 STRUCT 🧬                         */
/* ========================================================== */

#[derive(Clone, Debug)]
pub struct ToastData {
    pub id: ToastId,
    pub expiry: Option<u32>,
    pub message: String,
    pub progress: bool,
    pub dismissable: bool,
    /// Dismissal flag. This is an `ArcRwSignal` on purpose: toasts are built
    /// inside an `Effect` (the Desk announcement queue), and Leptos disposes any
    /// arena signal created during a run when that effect runs again. A
    /// reference-counted signal survives that cleanup, so the toast's timer and
    /// click handler can still flip it.
    pub clear_signal: ArcRwSignal<bool>,

    pub level: ToastLevel,
    pub position: ToastPosition,

    /// Optional action run when the toast is clicked, before it dismisses. Used
    /// by the Desk so a notification toast opens the notification it announces.
    pub on_click: Option<Callback<()>>,
}
