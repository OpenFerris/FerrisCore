use leptos::prelude::*;
use tw_merge::tw_merge;

/// A pulsing placeholder for content that is still loading.
///
/// Decorative by contract: `aria-hidden` keeps screen readers from announcing
/// a shape that carries no information, while the surrounding card owns the
/// `aria-busy`/status message.
#[component]
pub fn Skeleton(#[prop(into, optional)] class: String) -> impl IntoView {
    let merged_class = tw_merge!("animate-pulse rounded-md bg-muted", class);
    view! { <div class=merged_class aria-hidden="true" /> }
}
