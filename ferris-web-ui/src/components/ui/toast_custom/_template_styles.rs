pub const TEMPLATE_STYLES: &str = r#"
:root {
    --leptoaster-width: 320px;
    /* A toast must fit the narrowest phone (320px) with a hair of margin on
       each side; `80vw` left 32px unused there and still centred the text. */
    --leptoaster-max-width: calc(100vw - 24px);
    --leptoaster-z-index: 9999;

    --leptoaster-font-family: inherit;
    --leptoaster-font-size: 14px;
    --leptoaster-line-height: 20px;
    --leptoaster-font-weight: 600;

    --leptoaster-progress-height: 2px;

    --leptoaster-info-background-color: var(--card);
    --leptoaster-info-border-color: var(--border);
    --leptoaster-info-text-color: var(--card-foreground);

    --leptoaster-success-background-color: var(--success);
    --leptoaster-success-border-color: var(--success);
    --leptoaster-success-text-color: var(--success-foreground);

    --leptoaster-warn-background-color: var(--warning);
    --leptoaster-warn-border-color: var(--warning);
    --leptoaster-warn-text-color: var(--warning-foreground);

    --leptoaster-error-background-color: var(--destructive);
    --leptoaster-error-border-color: var(--destructive);
    --leptoaster-error-text-color: var(--destructive-foreground);
}

/* The Desk's mobile bottom bar is 4rem tall: a bottom toast must sit above it
   (and above the home indicator) instead of covering the navigation. */
@media (max-width: 767px) {
    .leptoaster-stack-container-bottom {
        margin-bottom: calc(4rem + 0.75rem + env(safe-area-inset-bottom, 0px)) !important;
    }
}

.leptoaster-stack-container-bottom:hover > div,
.leptoaster-stack-container-top:hover > div {
    opacity: 1 !important;
    transform: translateY(0) scaleX(1) !important;
    transition-delay: 0s !important;
}

.leptoaster-stack-container-bottom > div:nth-last-child(1),
.leptoaster-stack-container-top > div:nth-child(1) {
    z-index: 9999;
}

.leptoaster-stack-container-bottom > div:nth-last-child(2),
.leptoaster-stack-container-top > div:nth-child(2) {
    z-index: 9998;
}

.leptoaster-stack-container-bottom > div:nth-last-child(2) {
    transform: translateY(62px) scaleX(0.98);
}

.leptoaster-stack-container-top > div:nth-child(2) {
    transform: translateY(-62px) scaleX(0.98);
}

.leptoaster-stack-container-bottom > div:nth-last-child(3),
.leptoaster-stack-container-top > div:nth-child(3) {
    z-index: 9997;
}

.leptoaster-stack-container-bottom > div:nth-last-child(3) {
    transform: translateY(124px) scaleX(0.96);
}

.leptoaster-stack-container-top > div:nth-child(3) {
    transform: translateY(-124px) scaleX(0.96);
}

.leptoaster-stack-container-bottom > div:nth-last-child(4),
.leptoaster-stack-container-top > div:nth-child(4) {
    z-index: 9996;
}

.leptoaster-stack-container-bottom > div:nth-last-child(4) {
    transform: translateY(186px) scaleX(0.94);
}

.leptoaster-stack-container-top > div:nth-child(4) {
    transform: translateY(-186px) scaleX(0.94);
}

.leptoaster-stack-container-bottom > div:nth-last-child(5),
.leptoaster-stack-container-top > div:nth-child(5) {
    z-index: 9995;
}

.leptoaster-stack-container-bottom > div:nth-last-child(5) {
    transform: translateY(248px) scaleX(0.92);
}

.leptoaster-stack-container-top > div:nth-child(5) {
    transform: translateY(-248px) scaleX(0.92);
}

.leptoaster-stack-container-bottom > div:nth-last-child(n+6),
.leptoaster-stack-container-top > div:nth-child(n+6) {
    opacity: 0;
}

@keyframes leptoaster-slide-in-left {
    from { left: calc((var(--leptoaster-width) + 12px * 2) * -1) }
    to { left: 0 }
}

@keyframes leptoaster-slide-out-left {
    from { left: 0 }
    to { left: calc((var(--leptoaster-width) + 12px * 2) * -1) }
}

@keyframes leptoaster-slide-in-right {
    from { right: calc((var(--leptoaster-width) + 12px * 2) * -1) }
    to { right: 0 }
}

@keyframes leptoaster-slide-out-right {
    from { right: 0 }
    to { right: calc((var(--leptoaster-width) + 12px * 2) * -1) }
}

@keyframes leptoaster-progress {
    from { width: 100%; }
    to { width: 0; }
}
"#;
