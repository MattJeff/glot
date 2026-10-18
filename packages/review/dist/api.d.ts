import type { ReviewOptions } from "./types";
export type Actor = {
    user_id: string | null;
    link_id: string | null;
    name: string;
};
export type Reaction = {
    emoji: string;
    count: number;
    mine: boolean;
};
export type Issue = {
    code?: string;
    severity: "error" | "warning";
    message: string;
};
export type Anchor = {
    key?: string | null;
    key_source?: "attribute" | "marker" | "text" | "manual" | null;
    selector: string;
    text: string;
    url: string;
    locale: string;
    viewport: {
        w: number;
        h: number;
        dpr?: number;
    };
    rect?: {
        x: number;
        y: number;
        w: number;
        h: number;
    };
    point?: {
        x: number;
        y: number;
    };
    dom?: string;
};
export type Comment = {
    id: string;
    author: Actor;
    body: string;
    body_html: string;
    created_at: string;
    edited_at: string | null;
};
export type Ann = {
    id: string;
    status: "open" | "in_progress" | "resolved" | "wontfix";
    anchor: Anchor;
    route: string;
    locale: string;
    key: string | null;
    source: string | null;
    translation: {
        value: string;
        status: string;
    } | null;
    body: string;
    body_html: string;
    suggestion: string | null;
    author: Actor;
    comment_count: number;
    attachments: {
        id: string;
    }[];
    reactions: Reaction[];
    approvals: number;
    created_at: string;
    updated_at: string;
    comments?: Comment[];
};
export type Variant = {
    value: string;
    back_translation: string | null;
    issues: Issue[];
    valid: boolean;
};
export type Proposal = {
    id: number;
    text: string;
    explanation: string;
    variants: Record<string, Variant>;
    valid: boolean;
};
export type Rephrased = {
    rewrite_id: string;
    proposals: Proposal[];
    rejected: {
        text: string;
        reason: string[];
    }[];
};
export type Translation = {
    value: string;
    status: string;
    issues: Issue[];
};
export type TextEntry = {
    key: string;
    source: string;
    source_locale: string;
    description: string | null;
    values: Record<string, Translation>;
};
export type Context = {
    project: {
        slug: string;
        name: string;
        source_locale: string;
        target_locales: string[];
    };
    access: "read" | "comment" | "edit" | "member";
    me: Actor;
    expires_at: string | null;
};
export type ReviewEvent = {
    id: number;
    annotation_id: string | null;
    kind: string;
    actor: Actor;
    data: any;
};
export declare class ApiError extends Error {
    status: number;
    code: string;
    details?: any | undefined;
    constructor(status: number, code: string, message: string, details?: any | undefined);
}
/** `guard` runs before every write (the widget uses it to ask a review-link reviewer for their name). */
export declare function makeApi(o: ReviewOptions & {
    token: string;
}, name: () => string | undefined, guard?: () => void): {
    context: () => Promise<Context>;
    list: (route: string, locale: string) => Promise<Ann[]>;
    get: (id: string) => Promise<Ann>;
    create: (anchor: Anchor, body: string, suggestion?: string) => Promise<Ann>;
    patch: (id: string, b: Partial<{
        status: string;
        body: string;
        suggestion: string | null;
    }>) => Promise<Ann>;
    del: (id: string) => Promise<void>;
    comment: (id: string, body: string) => Promise<Comment>;
    editComment: (id: string, cid: string, body: string) => Promise<Comment>;
    delComment: (id: string, cid: string) => Promise<void>;
    react: (id: string, emoji: string, comment_id?: string) => Promise<{
        reactions: Reaction[];
    }>;
    attach: (id: string, img: Blob) => Promise<unknown>;
    shot: (id: string, aid: string) => string;
    apply: (id: string, value?: string) => Promise<Ann>;
    /** RV4: 2-3 proposals, each declined and validated in every project locale. */
    rephrase: (id: string, preset: string, segment?: string) => Promise<Rephrased>;
    applyRewrite: (id: string, rewrite_id: string, proposal_id: number) => Promise<unknown>;
    text: (key: string) => Promise<TextEntry>;
    /** dry_run: live validation. Throws ApiError 422 (details.results) when a locale has an error. */
    put: (key: string, values: Record<string, string>, dry_run?: boolean) => Promise<{
        results: Record<string, {
            issues: Issue[];
        }>;
    }>;
    revert: (key: string, locale: string) => Promise<{
        translation: Translation;
    }>;
    unset: (key: string, locale: string) => Promise<void>;
    createKey: (source: string, anchor: Anchor, key?: string) => Promise<TextEntry>;
    events: (on: (e: ReviewEvent) => void) => EventSource;
};
export type Api = ReturnType<typeof makeApi>;
