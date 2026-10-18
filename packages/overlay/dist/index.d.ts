export interface OverlayOptions {
    /** glot dev URL. Default: origin of the <script src> serving it, else http://127.0.0.1:4317 */
    url?: string;
    /** Token printed by `glot dev`. Else ?glot_token=, data-token, localStorage, or asked in the UI. */
    token?: string;
    /** Cookie used to switch the app locale (next-intl default). */
    localeCookie?: string;
}
export declare function init(opts?: OverlayOptions): void;
