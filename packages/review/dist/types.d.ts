export type ReviewOptions = {
    /** Project slug. */
    project: string;
    /** glot server origin (default: same origin). */
    endpoint?: string;
    /** Review token (default: `?glot_review=` of the review link, kept for the tab). */
    token?: string;
    /** Locale the page renders (default: <html lang>). */
    locale?: string;
    /** glot web app URL, for the "Open in glot" button (e.g. https://app.glot.dev). */
    appUrl?: string;
    /** Origin of the glot Review page embedding the site in an iframe (postMessage protocol). */
    appOrigin?: string;
    /** Widget language and direction (default: French or English from the browser; rtl for ar/he/fa/ur). */
    lang?: string;
    dir?: "ltr" | "rtl";
    /** Widget strings (default: French if the reviewer's browser is French, else English). */
    labels?: Partial<Labels>;
};
export declare const EN: {
    bar: string;
    move: string;
    comment: string;
    list: string;
    filter: string;
    open: string;
    resolved: string;
    all: string;
    saas: string;
    quit: string;
    pickHelp: string;
    noKey: string;
    newTitle: string;
    close: string;
    body: string;
    propose: string;
    rephrase: string;
    shorter: string;
    clearer: string;
    formal: string;
    warmer: string;
    segment: string;
    preview: string;
    valid: string;
    send: string;
    submit: string;
    apply: string;
    applied: string;
    reply: string;
    edit: string;
    del: string;
    save: string;
    cancel: string;
    resolve: string;
    reopen: string;
    orphan: string;
    empty: string;
    loading: string;
    added: string;
    error: string;
    expired: string;
    you: string;
    proposal: string;
    tabThread: string;
    tabText: string;
    tabAll: string;
    s_open: string;
    s_in_progress: string;
    s_resolved: string;
    s_wontfix: string;
    saveText: string;
    saved: string;
    revert: string;
    reverted: string;
    reset: string;
    unset: string;
    unsetConfirm: string;
    unsetDone: string;
    createKey: string;
    keyName: string;
    keyCreated: string;
    noKeyHelp: string;
    ph: string;
    phBroken: string;
    yourName: string;
    approve: string;
    readOnly: string;
    source: string;
    missing: string;
    noApply: string;
    suggestion: string;
    proposals: string;
    segmentLabel: string;
    segmentHelp: string;
    applyAll: string;
    validIn: string;
    nameNeeded: string;
};
export type Labels = typeof EN;
export declare const FR: Labels;
