use icons::{ChevronRight, Ellipsis};
use leptos::prelude::*;
use leptos_ui::clx;

use ferris_web_shared::url::safe_href;

mod components {
    use super::*;
    clx! {Breadcrumb, nav, ""}
    clx! {BreadcrumbList, ol, "flex flex-wrap gap-1 items-center text-sm break-words sm:gap-2 text-muted-foreground"}
    clx! {BreadcrumbItem, li, "inline-flex gap-1 items-center [&_svg:not([class*='size-'])]:size-4"}
    clx! {BreadcrumbLink, a, "transition-colors hover:text-foreground"}
    clx! {RootSeparator, li, "[&>svg]:size-3.5 [&_svg:not([class*='size-'])]:size-4"}
    clx! {RootPage, span, "font-normal text-foreground"}
    // A phone gets a real tap box: the ellipsis glyph alone was a 16px target.
    clx! {RootEllipsisBtn, button, "flex items-center gap-1 max-md:min-h-10 max-md:min-w-10 max-md:justify-center"}
    clx! {RootEllipsis, span, "flex items-center justify-center size-4"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn BreadcrumbSeparator(
    #[prop(into, optional)] class: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <RootSeparator class=class attr:role="presentation" attr:aria-hidden="true">
            {match children {
                Some(c) => c().into_any(),
                None => view! { <ChevronRight /> }.into_any(),
            }}
        </RootSeparator>
    }
}

#[component]
pub fn BreadcrumbPage(#[prop(into, optional)] class: String, children: Children) -> impl IntoView {
    // TODO. aria_disabled
    view! {
        <RootPage class=class attr:role="link" attr:aria-disabled="true" attr:aria-current="page">
            {children()}
        </RootPage>
    }
}

#[component]
pub fn BreadcrumbEllipsis(#[prop(into, optional)] class: String) -> impl IntoView {
    // TODO. data_state
    view! {
        <RootEllipsisBtn attr:r#type="button" attr:aria-label=crate::i18n::t("ui.breadcrumb.show_more") attr:aria-haspopup="menu" attr:aria-expanded="false" attr:data-state="closed">
            <RootEllipsis attr:role="presentation" attr:aria-hidden="true">
                <Ellipsis class=class />
                <span class="hidden">{crate::i18n::t("ui.breadcrumb.more")}</span>
            </RootEllipsis>
            <span class="hidden">{crate::i18n::t("ui.breadcrumb.open_menu")}</span>
        </RootEllipsisBtn>
    }
}

/* ========================================================== */
/*                     ✨ BREADCRUMB TRAIL ✨                 */
/* ========================================================== */

/// One crumb in a trail.
///
/// The trail is built by the caller (the Desk app, which owns the route table):
/// only it knows which ancestors are real pages. `href` is therefore `None` for
/// an ancestor that is a label but not a destination — a module with no index
/// page, a section heading — and such a crumb renders as plain text. The page
/// on screen sets `is_current` and is never a link.
#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub label: String,
    pub href: Option<String>,
    pub is_current: bool,
}

/// Render a trail: separator-separated crumbs, the current one as a page and
/// every navigable ancestor as a link.
pub fn build_breadcrumb_view(crumbs: Vec<Crumb>) -> impl IntoView {
    view! {
        <Breadcrumb>
            <BreadcrumbList>
                {crumbs.into_iter().enumerate().map(|(i, c)| {
                    let Crumb { label, href, is_current } = c;

                    view! {
                        {(i > 0).then(|| view! { <BreadcrumbSeparator /> })}
                        <BreadcrumbItem>
                            {if is_current {
                                view! {
                                    <BreadcrumbPage class="max-w-[220px] truncate">{label}</BreadcrumbPage>
                                }.into_any()
                            } else if let Some(href) = href.and_then(|href| safe_href(&href)) {
                                view! {
                                    <BreadcrumbLink attr:href={href} class="max-w-[160px] truncate">{label}</BreadcrumbLink>
                                }.into_any()
                            } else {
                                view! {
                                    <span class="max-w-[160px] truncate">{label}</span>
                                }.into_any()
                            }}
                        </BreadcrumbItem>
                    }
                }).collect_view()}
            </BreadcrumbList>
        </Breadcrumb>
    }
}
