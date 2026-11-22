//! Inline-script focus trap shared by every modal overlay.
//!
//! The dialogs, sheets and the command palette each carry a small inline
//! `<script>` (there is no client-side JS bundle to import from). Before this
//! module, each one copied its own "close on Escape" listener and none trapped
//! Tab, marked the background inert or restored focus, so a keyboard user could
//! tab into the page behind the modal and lose their place when it closed.
//!
//! [`FOCUS_TRAP_JS`] defines `window.__ferrisFocusTrap` with two methods:
//!
//! - `enter(panel)`: remembers `document.activeElement`, marks every sibling
//!   along the panel's ancestor chain `inert` + `aria-hidden`, and moves focus
//!   into the panel (first focusable element, or the panel itself).
//! - `leave(panel)`: undoes the marks in reverse and returns focus to the
//!   element that opened the overlay.
//!
//! Both are idempotent, so a component may call `enter` from the state change
//! and again from a click handler without stacking listeners. The host script
//! embeds the constant with `{FOCUS_TRAP_JS}`; the text contains `{`/`}`, but a
//! named `format!` argument is inserted verbatim and never re-parsed.

/// JS source that installs `window.__ferrisFocusTrap`.
pub const FOCUS_TRAP_JS: &str = r#"
window.__ferrisFocusTrap = window.__ferrisFocusTrap || (() => {
    const SELECTOR = 'a[href],area[href],button:not([disabled]),input:not([disabled]):not([type="hidden"]),select:not([disabled]),textarea:not([disabled]),iframe,object,embed,[tabindex]:not([tabindex="-1"]),[contenteditable="true"]';
    const state = new WeakMap();

    const focusables = (panel) => Array.from(panel.querySelectorAll(SELECTOR)).filter((element) => {
        if (element.hidden || element.getAttribute('aria-hidden') === 'true') return false;
        return Boolean(element.offsetWidth || element.offsetHeight || element.getClientRects().length);
    });

    const markBackground = (panel) => {
        const marked = [];
        let node = panel;
        while (node && node !== document.body) {
            const parent = node.parentElement;
            if (!parent) break;
            for (const sibling of Array.from(parent.children)) {
                if (sibling === node) continue;
                // The backdrop is a sibling of the panel and must stay
                // clickable: an inert backdrop turns "click outside to close"
                // into a dead surface.
                if (sibling.hasAttribute('data-ferris-backdrop')) continue;
                if (!(sibling instanceof HTMLElement)) continue;
                marked.push({
                    element: sibling,
                    inert: sibling.inert,
                    ariaHidden: sibling.getAttribute('aria-hidden'),
                });
                sibling.inert = true;
                sibling.setAttribute('aria-hidden', 'true');
            }
            node = parent;
        }
        return marked;
    };

    const unmarkBackground = (marked) => {
        for (const entry of marked) {
            entry.element.inert = entry.inert;
            if (entry.ariaHidden === null) {
                entry.element.removeAttribute('aria-hidden');
            } else {
                entry.element.setAttribute('aria-hidden', entry.ariaHidden);
            }
        }
    };

    const focusInto = (panel) => {
        if (!panel.hasAttribute('tabindex')) panel.setAttribute('tabindex', '-1');
        const target = focusables(panel)[0] || panel;
        try {
            target.focus({ preventScroll: true });
        } catch (_) {
            target.focus();
        }
    };

    const enter = (panel) => {
        if (!panel || state.has(panel)) return;
        const previous = document.activeElement;
        const marked = markBackground(panel);
        const onKeydown = (event) => {
            if (event.key !== 'Tab') return;
            const items = focusables(panel);
            if (items.length === 0) {
                event.preventDefault();
                panel.focus();
                return;
            }
            const first = items[0];
            const last = items[items.length - 1];
            const active = document.activeElement;
            if (event.shiftKey) {
                if (active === first || active === panel || !panel.contains(active)) {
                    event.preventDefault();
                    last.focus();
                }
            } else if (active === last || !panel.contains(active)) {
                event.preventDefault();
                first.focus();
            }
        };
        document.addEventListener('keydown', onKeydown, true);
        state.set(panel, { previous, marked, onKeydown });
        // A frame lets the transition start before focus scrolls the panel.
        requestAnimationFrame(() => focusInto(panel));
    };

    const leave = (panel) => {
        if (!panel) return;
        const previousState = state.get(panel);
        if (!previousState) return;
        document.removeEventListener('keydown', previousState.onKeydown, true);
        unmarkBackground(previousState.marked);
        state.delete(panel);
        const previous = previousState.previous;
        if (previous && previous.isConnected && typeof previous.focus === 'function') {
            try {
                previous.focus({ preventScroll: true });
            } catch (_) {
                previous.focus();
            }
        }
    };

    return { enter, leave };
})();
"#;
