export interface BadgeOptions {
    /** Badge text (pass it translated — e.g. from your fallback messages). */
    label?: string;
    /** "Improve" link text. */
    improveLabel?: string;
    /** Where "Improve" goes (glot overlay / feedback form). Without it, the link only dispatches `glot:improve`. */
    improveUrl?: string;
    locale?: string;
    /** Model download progress 0..1 while preparing; the badge shows a percentage. */
    progress?: number;
    /** Shown when the model must be downloaded: clicking calls `onActivate` (inside the user gesture). */
    activateLabel?: string;
    onActivate?: () => void;
}
/** Show/update the badge. Returns a remover. No-op without a DOM. */
export declare function showBadge(o?: BadgeOptions): () => void;
