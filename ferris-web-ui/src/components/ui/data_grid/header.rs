use super::*;

/// A header cell with sorting and pinning dropdown menu.
/// Works with any column type that implements `PinnableColumn`.

#[component]
pub fn PinnableSortableHeaderCell<C>(
    column: C,
    #[prop(into)] label: String,
    sort_signal: RwSignal<SortDirection>,
    pinned_columns_signal: RwSignal<HashSet<C>>,
    #[prop(optional)] visible_columns_signal: Option<RwSignal<HashSet<String>>>,
    #[prop(default = false)] is_pinned: bool,
) -> impl IntoView
where
    C: PinnableColumn + AsRef<str> + Send + Sync + 'static,
{
    let width = get_column_width(column);
    let column_name = column.as_ref().to_string();

    view! {
        <DropdownMenu>
            <DropdownMenuTrigger class="flex gap-2 justify-between items-center p-2 w-full h-full text-sm rounded-none border-0 shadow-none data-[state=open]:bg-accent/40 [&_svg]:size-4 hover:bg-accent/40">
                <div class="flex flex-1 gap-1.5 items-center min-w-0">
                    <span class="truncate">{label}</span>
                </div>
                <ChevronDown class="shrink-0 text-muted-foreground" />
            </DropdownMenuTrigger>
            <DropdownMenuContent>
                <DropdownMenuRadioGroup value=sort_signal>
                    <DropdownMenuRadioItem value=SortDirection::Asc>
                        <ArrowUpNarrowWide class="text-muted-foreground" />
                        <span>"Sort asc"</span>
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value=SortDirection::Desc>
                        <ArrowDownWideNarrow class="text-muted-foreground" />
                        <span>"Sort desc"</span>
                    </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
                {move || {
                    (sort_signal.get() != SortDirection::None)
                        .then(|| {
                            view! {
                                <DropdownMenuItem on:click=move |_| sort_signal.set(SortDirection::None)>
                                    <DropdownMenuAction>
                                        <CircleX class="text-muted-foreground" />
                                        <span>"Remove sort"</span>
                                    </DropdownMenuAction>
                                </DropdownMenuItem>
                            }
                        })
                }}
                <DropdownMenuSeparator />
                <DropdownMenuGroup>
                    {if is_pinned {
                        view! {
                            <DropdownMenuItem on:click=move |_| {
                                pinned_columns_signal
                                    .update(|p| {
                                        p.remove(&column);
                                    })
                            }>
                                <DropdownMenuAction>
                                    <PanelLeftClose class="text-muted-foreground" />
                                    <span>"Unpin from left"</span>
                                </DropdownMenuAction>
                            </DropdownMenuItem>
                        }
                            .into_any()
                    } else {
                        view! {
                            <DropdownMenuItem on:click=move |_| {
                                pinned_columns_signal
                                    .update(|p| {
                                        p.insert(column);
                                    })
                            }>
                                <DropdownMenuAction>
                                    <PanelLeft class="text-muted-foreground" />
                                    <span>"Pin to left"</span>
                                </DropdownMenuAction>
                            </DropdownMenuItem>
                        }
                            .into_any()
                    }}
                </DropdownMenuGroup>
                {visible_columns_signal
                    .map(|signal| {
                        let column_name = column_name.clone();
                        view! {
                            <DropdownMenuGroup>
                                <DropdownMenuItem on:click=move |_| {
                                    signal
                                        .update(|v| {
                                            v.remove(&column_name);
                                        })
                                }>
                                    <DropdownMenuAction>
                                        <EyeOff class="text-muted-foreground" />
                                        <span>"Hide Column"</span>
                                    </DropdownMenuAction>
                                </DropdownMenuItem>
                            </DropdownMenuGroup>
                        }
                    })}
            </DropdownMenuContent>
        </DropdownMenu>
        <TableSeparator valuenow=width />
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Editable cell content component.
/// Double-click to edit, Enter to save, Escape to cancel.
#[component]
pub fn EditableCellContent<C: DataGridColumn + 'static>(
    row_idx: usize,
    col: C,
    #[prop(into)] value: String,
    #[prop(optional)] on_save: Option<Callback<(usize, C, String)>>,
) -> impl IntoView {
    let ctx = expect_context::<CellEditContext<C>>();

    let value = StoredValue::new(value);
    let on_save = StoredValue::new(on_save);

    let is_editing = Signal::derive(move || ctx.is_editing(row_idx, col));

    view! {
        <Show
            when=move || is_editing.get()
            fallback=move || {
                view! {
                    <GridCellContent
                        class="cursor-text"
                        on:dblclick=move |_| {
                            ctx.start_edit(row_idx, col, value.get_value());
                        }
                    >
                        {value.get_value()}
                    </GridCellContent>
                }
            }
        >
            <input
                type="text"
                class="py-0 px-0 w-full h-full text-sm bg-transparent border-none outline-none focus:ring-0"
                prop:value=move || ctx.edit_value.get()
                on:input=move |ev| ctx.edit_value.set(event_target_value(&ev))
                on:blur=move |_| {
                    if let Some((row, column, new_value)) = ctx.finish_edit() {
                        on_save
                            .with_value(|cb| {
                                if let Some(cb) = cb {
                                    cb.run((row, column, new_value));
                                }
                            });
                    }
                }
                on:keydown=move |ev: KeyboardEvent| {
                    match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            if let Some((row, column, new_value)) = ctx.finish_edit() {
                                on_save
                                    .with_value(|cb| {
                                        if let Some(cb) = cb {
                                            cb.run((row, column, new_value));
                                        }
                                    });
                            }
                        }
                        "Escape" => {
                            ev.prevent_default();
                            ctx.cancel_edit();
                        }
                        _ => {}
                    }
                }
                autofocus=true
            />
        </Show>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn DataGridToolbar(children: Children, #[prop(optional, into)] class: String) -> impl IntoView {
    let merged_class = tw_merge!("flex gap-4 justify-between items-center mb-4", class);

    view! {
        <div data-name="DataGridToolbar" role="toolbar" aria-orientation="horizontal" class=merged_class>
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Generic grid header component that works with any Column type.
/// Handles the select-all checkbox, pinned headers, and non-pinned headers.
#[component]
pub fn GenericGridHeader<C>(
    row_count_signal: Signal<usize>,
    selected_count_signal: Signal<usize>,
    handle_select_all: Callback<bool>,
    sort_signals: StoredValue<HashMap<C, RwSignal<SortDirection>>>,
    pinned_columns_signal: RwSignal<HashSet<C>>,
    visible_columns_signal: RwSignal<HashSet<String>>,
) -> impl IntoView
where
    C: DataGridColumn + std::fmt::Display + 'static,
{
    view! {
        <div role="rowgroup" data-slot="grid-header" class="grid sticky top-0 z-10 border-b bg-background">
            <div role="row" aria-rowindex="1" data-slot="grid-header-row" tabindex="-1" class="flex w-full">
                // Select header (always sticky)
                <GridSelectHeaderCell>
                    <div class="py-1.5 px-3 size-full">
                        <Checkbox
                            checked=Signal::derive(move || {
                                let row_count = row_count_signal.get();
                                row_count > 0 && selected_count_signal.get() == row_count
                            })
                            on_checked_change=handle_select_all
                            aria_label="Seleccionar todo"
                        />
                    </div>
                </GridSelectHeaderCell>

                // Pinned headers (dynamic loop)
                <For
                    each=move || get_pinned_visible_columns(pinned_columns_signal, visible_columns_signal)
                    key=|(col, _)| *col
                    children=move |(col, _width)| {
                        let sort_signal = sort_signals.with_value(|s| s.get(&col).copied());
                        let Some(sort_signal) = sort_signal else {
                            return view! { <div></div> }.into_any();
                        };
                        let left = pinned_columns_signal.with(|p| get_pinned_left_position(col, p));
                        let width = get_column_width(col);

                        view! {
                            <GridPinnedHeaderCell left=left width=width>
                                <PinnableSortableHeaderCell
                                    column=col
                                    label=col.to_string()
                                    sort_signal=sort_signal
                                    pinned_columns_signal=pinned_columns_signal
                                    visible_columns_signal=visible_columns_signal
                                    is_pinned=true
                                />
                            </GridPinnedHeaderCell>
                        }
                            .into_any()
                    }
                />

                // Non-pinned headers (dynamic loop)
                <For
                    each=move || C::pinnable_columns().to_vec()
                    key=|(col, _)| *col
                    children=move |(col, _width)| {
                        let sort_signal = sort_signals.with_value(|s| s.get(&col).copied());
                        let Some(sort_signal) = sort_signal else {
                            return view! { <div></div> }.into_any();
                        };
                        view! {
                            <GridHeaderCell
                                colindex=col.colindex()
                                column=col.css_safe_name()
                                visible=col.is_visible(pinned_columns_signal, visible_columns_signal)
                            >
                                <PinnableSortableHeaderCell
                                    column=col
                                    label=col.to_string()
                                    sort_signal=sort_signal
                                    pinned_columns_signal=pinned_columns_signal
                                    visible_columns_signal=visible_columns_signal
                                />
                            </GridHeaderCell>
                        }
                            .into_any()
                    }
                />
            </div>
        </div>
    }
}
