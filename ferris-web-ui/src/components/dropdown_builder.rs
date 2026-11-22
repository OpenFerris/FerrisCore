use std::borrow::Cow;

use icons::ChevronsUpDown;
use leptos::prelude::*;

use crate::components::ui::{
    button::{Button, ButtonSize, ButtonVariant},
    dropdown_menu::*,
    icon::Icon,
};

/// Define las distintas variantes que puede tener un elemento del menú.
pub enum DropdownItem {
    /// Acción normal (clic)
    Action {
        label: String,
        icon: Option<Cow<'static, str>>,
        trailing_icon: Option<Cow<'static, str>>,
        destructive: bool,
        disabled: bool,
        on_click: Callback<()>,
    },
    /// Enlace (`<a href=...>`)
    Link {
        label: String,
        icon: Option<Cow<'static, str>>,
        trailing_icon: Option<Cow<'static, str>>,
        href: String,
        muted: bool,
    },
    /// Submenú anidado
    Submenu {
        label: String,
        icon: Option<Cow<'static, str>>,
        items: Vec<DropdownItem>,
    },
    /// Submenú cuyo disparador es una vista propia (p. ej. un enlace que además
    /// despliega sus carpetas), en lugar del icono + etiqueta de [`Self::Submenu`].
    SubmenuCustom {
        trigger: AnyView,
        items: Vec<DropdownItem>,
    },
    /// Contenido visual / componente custom (ej. Toggle theme, Checkboxes)
    Custom {
        view: AnyView,
        on_click: Option<Callback<()>>,
    },
    /// Línea divisora
    Separator,
}

impl DropdownItem {
    pub fn action(label: impl Into<String>, on_click: Callback<()>) -> Self {
        Self::Action {
            label: label.into(),
            icon: None,
            trailing_icon: None,
            destructive: false,
            disabled: false,
            on_click,
        }
    }

    pub fn link(label: impl Into<String>, href: impl Into<String>) -> Self {
        Self::Link {
            label: label.into(),
            icon: None,
            trailing_icon: None,
            href: href.into(),
            muted: false,
        }
    }

    pub fn submenu(label: impl Into<String>, items: Vec<DropdownItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            icon: None,
            items,
        }
    }

    pub fn submenu_custom(trigger: impl IntoView, items: Vec<DropdownItem>) -> Self {
        Self::SubmenuCustom {
            trigger: trigger.into_any(),
            items,
        }
    }

    pub fn custom(view: impl IntoView, on_click: Option<Callback<()>>) -> Self {
        Self::Custom {
            view: view.into_any(),
            on_click,
        }
    }

    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn with_icon(mut self, icon: impl Into<Cow<'static, str>>) -> Self {
        match &mut self {
            Self::Action { icon: i, .. }
            | Self::Link { icon: i, .. }
            | Self::Submenu { icon: i, .. } => *i = Some(icon.into()),
            _ => {}
        }
        self
    }

    pub fn with_trailing_icon(mut self, icon: impl Into<Cow<'static, str>>) -> Self {
        match &mut self {
            Self::Action {
                trailing_icon: t, ..
            }
            | Self::Link {
                trailing_icon: t, ..
            } => *t = Some(icon.into()),
            _ => {}
        }
        self
    }

    pub fn destructive(mut self) -> Self {
        if let Self::Action { destructive, .. } = &mut self {
            *destructive = true;
        }
        self
    }

    pub fn disabled(mut self) -> Self {
        if let Self::Action { disabled, .. } = &mut self {
            *disabled = true;
        }
        self
    }

    pub fn muted(mut self) -> Self {
        if let Self::Link { muted, .. } = &mut self {
            *muted = true;
        }
        self
    }
}

pub enum DropdownTriggerSpec {
    /// Disparador tipo botón básico
    Simple {
        label: String,
        icon: Option<Cow<'static, str>>,
        selectable: bool,
    },
    /// Disparador avanzado con ícono, título y subtítulo (módulo)
    Custom {
        view: AnyView,
        // on_click: Option<Callback<()>>,
    },
}

pub struct DropdownSpec {
    pub align: DropdownMenuAlign,
    pub trigger: DropdownTriggerSpec,
    pub items: Vec<DropdownItem>,
}

#[component]
pub fn DropdownBuilder(spec: DropdownSpec) -> impl IntoView {
    let DropdownSpec {
        align,
        trigger,
        items,
    } = spec;

    view! {
        <DropdownMenu align=align>
            // Renderizado del Trigger según la variante
            {match trigger {
                DropdownTriggerSpec::Simple { label, icon, selectable } => {
                    let text = label.clone();
                    view! {
                        <DropdownMenuTrigger as_child=true>
                            <Button
                                variant=ButtonVariant::Outline
                                size=ButtonSize::Sm
                                attr:aria-label=label
                            >
                                {icon.map(|name| view! { <Icon name=name /> })}
                                {text}
                                {selectable.then(|| view! { <ChevronsUpDown /> })}
                            </Button>
                        </DropdownMenuTrigger>
                    }.into_any()
                }

                DropdownTriggerSpec::Custom { view } => {
                    view! {
                        <DropdownMenuTrigger as_child=true>
                            {view}
                        </DropdownMenuTrigger>
                    }.into_any()
                }
            }}

            // Renderizado recursivo/iterativo del contenido
            <DropdownMenuContent>
                <DropdownMenuGroup>
                    {items.into_iter().map(render_dropdown_item).collect_view()}
                </DropdownMenuGroup>
            </DropdownMenuContent>
        </DropdownMenu>
    }
}

fn render_dropdown_item(item: DropdownItem) -> AnyView {
    match item {
        DropdownItem::Action {
            label,
            icon,
            trailing_icon,
            destructive,
            disabled,
            on_click,
        } => {
            let variant = if destructive {
                DropdownMenuActionVariant::Destructive
            } else {
                DropdownMenuActionVariant::Default
            };

            view! {
                <DropdownMenuItem>
                    <DropdownMenuAction variant=variant on_click=on_click disabled=disabled>
                        {icon.map(|n| view! { <Icon name=n /> })}
                        <DropdownMenuItemLabel>{label}</DropdownMenuItemLabel>
                        {trailing_icon.map(|n| view! {
                            <DropdownMenuItemTrailing><Icon name=n /></DropdownMenuItemTrailing>
                        })}
                    </DropdownMenuAction>
                </DropdownMenuItem>
            }
            .into_any()
        }

        DropdownItem::Link {
            label,
            icon,
            trailing_icon,
            href,
            muted,
        } => {
            let variant = if muted {
                DropdownMenuActionVariant::Muted
            } else {
                DropdownMenuActionVariant::Default
            };

            view! {
                <DropdownMenuItem>
                    <DropdownMenuAction variant=variant href=href>
                        {icon.map(|n| view! { <Icon name=n /> })}
                        <DropdownMenuItemLabel>{label}</DropdownMenuItemLabel>
                        {trailing_icon.map(|n| view! {
                            <DropdownMenuItemTrailing><Icon name=n /></DropdownMenuItemTrailing>
                        })}
                    </DropdownMenuAction>
                </DropdownMenuItem>
            }
            .into_any()
        }

        DropdownItem::Submenu { label, icon, items } => view! {
            <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                    {icon.map(|n| view! { <Icon name=n /> })}
                    <DropdownMenuItemLabel>{label}</DropdownMenuItemLabel>
                </DropdownMenuSubTrigger>

                <DropdownMenuSubContent>
                    <DropdownMenuGroup>
                        {items.into_iter().map(render_dropdown_item).collect_view()}
                    </DropdownMenuGroup>
                </DropdownMenuSubContent>
            </DropdownMenuSub>
        }
        .into_any(),

        DropdownItem::SubmenuCustom { trigger, items } => view! {
            <DropdownMenuSub>
                <DropdownMenuSubTrigger>{trigger}</DropdownMenuSubTrigger>

                <DropdownMenuSubContent>
                    <DropdownMenuGroup>
                        {items.into_iter().map(render_dropdown_item).collect_view()}
                    </DropdownMenuGroup>
                </DropdownMenuSubContent>
            </DropdownMenuSub>
        }
        .into_any(),

        DropdownItem::Custom { view, on_click } => match on_click {
            Some(on_click) => view! {
                <DropdownMenuCustomItem attr:data-dropdown-close="true" on_click=on_click>
                    {view}
                </DropdownMenuCustomItem>
            }
            .into_any(),
            None => view! { <DropdownMenuCustomItem>{view}</DropdownMenuCustomItem> }.into_any(),
        },

        DropdownItem::Separator => view! {
            <DropdownMenuSeparator />
        }
        .into_any(),
    }
}
