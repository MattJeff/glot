#!/usr/bin/env python3
"""Generates bench/data/messages.jsonl (CC0): 500 hard English UI messages for glot-bench.

Hand-written patterns modelled on real open-source app catalogs (issue trackers, chat, billing,
file sharing, CI dashboards), expanded over hand-picked nouns. Deterministic, deduplicated.
Run: python3 bench/data/gen.py
"""
import json, re
from pathlib import Path

OUT = Path(__file__).with_name("messages.jsonl")
TERMS = ["Workspace", "Invoice", "Project", "Dashboard", "Billing"]
DNT = ["glot", "GitHub", "Slack", "Stripe", "Acme", "OAuth", "API", "Markdown"]

BUTTON_96 = {"max_width_px": 96, "max_lines": 1, "font": {"family": "Inter", "size_px": 14, "weight": 500}}
BUTTON_120 = {"max_width_px": 120, "max_lines": 1, "font": {"family": "Inter", "size_px": 14, "weight": 500}}
TAB_80 = {"max_width_px": 80, "max_lines": 1, "font": {"family": "Inter", "size_px": 13, "weight": 500}}
CARD_2L = {"max_width_px": 180, "max_lines": 2, "font": {"family": "Inter", "size_px": 14, "weight": 400}}

out, seen = [], set()


def add(cat, source, desc, role="text", fit=None):
    if source in seen:
        return
    seen.add(source)
    m = {"id": f"{cat}.{sum(1 for x in out if x['category'] == cat) + 1:03d}", "category": cat,
         "source": source, "description": desc, "ui_role": role}
    words = set(re.findall(r"[A-Za-z]+", source))
    g = [t for t in TERMS + DNT if t in words]
    if g:
        m["glossary"] = g
    if fit:
        m["fit"] = fit
    out.append(m)


# (singular, plural, context) — countable things in SaaS UIs.
NOUNS = [
    ("file", "files", "file browser"), ("comment", "comments", "issue thread"),
    ("member", "members", "team settings"), ("task", "tasks", "kanban board"),
    ("message", "messages", "chat sidebar"), ("seat", "seats", "plan page"),
    ("review", "reviews", "pull request"), ("download", "downloads", "release page"),
    ("notification", "notifications", "bell menu"), ("invitation", "invitations", "team page"),
    ("error", "errors", "build log"), ("warning", "warnings", "linter panel"),
    ("result", "results", "search page"), ("attachment", "attachments", "email composer"),
    ("subscriber", "subscribers", "newsletter stats"), ("day", "days", "trial banner"),
    ("hour", "hours", "time tracking"), ("minute", "minutes", "meeting card"),
    ("step", "steps", "onboarding checklist"), ("credit", "credits", "usage meter"),
    ("photo", "photos", "album view"), ("deployment", "deployments", "CI dashboard"),
    ("key", "keys", "API keys page"), ("draft", "drafts", "editor sidebar"),
    ("reply", "replies", "forum thread"), ("vote", "votes", "poll widget"),
    ("follower", "followers", "profile header"), ("item", "items", "shopping cart"),
    ("ticket", "tickets", "support inbox"), ("event", "events", "calendar"),
    ("page", "pages", "documentation site"), ("tag", "tags", "label picker"),
    ("branch", "branches", "repository view"), ("commit", "commits", "history tab"),
    ("contact", "contacts", "CRM list"), ("order", "orders", "store admin"),
]

# 1. Plurals with `#`, `=0`, `=1`, offset.
for s, p, ctx in NOUNS:
    add("plural", f"{{count, plural, one {{# {s}}} other {{# {p}}}}}", f"Counter in the {ctx}.", "label")
    add("plural", f"{{count, plural, =0 {{No {p} yet}} one {{# {s}}} other {{# {p}}}}}",
        f"Empty-state aware counter in the {ctx}.", "label")
for s, p, ctx in NOUNS[:16]:
    add("plural", f"You have {{count, plural, =0 {{no new {p}}} one {{one new {s}}} other {{# new {p}}}}}.",
        f"Summary sentence in the {ctx}.", "text")
for s, p, ctx in NOUNS[:16]:
    add("plural_offset",
        f"{{count, plural, offset:1 =0 {{Nobody added a {s}}} =1 {{{{name}} added a {s}}} one {{{{name}} and # other added {p}}} other {{{{name}} and # others added {p}}}}}",
        f"Activity feed line in the {ctx}; `name` is the first person.", "text")

# 2. select (gender / role / status).
SELECT = [
    ("{gender, select, female {She replied to your comment} male {He replied to your comment} other {They replied to your comment}}", "Notification about a reply; gender of the author."),
    ("{gender, select, female {{name} updated her profile} male {{name} updated his profile} other {{name} updated their profile}}", "Activity feed; gender of the user."),
    ("{gender, select, female {Invite her to the team} male {Invite him to the team} other {Invite them to the team}}", "Suggestion card in team settings."),
    ("{gender, select, female {{name} is now an admin. She can manage billing.} male {{name} is now an admin. He can manage billing.} other {{name} is now an admin. They can manage billing.}}", "Toast after a role change."),
    ("{role, select, owner {Owner} admin {Admin} member {Member} guest {Guest} other {Unknown role}}", "Role badge in the members list."),
    ("{status, select, open {Open} closed {Closed} merged {Merged} draft {Draft} other {Unknown}}", "Pull request status pill."),
    ("{status, select, paid {Paid} due {Due} overdue {Overdue} void {Void} other {Pending}}", "Payment status chip on an invoice row."),
    ("{plan, select, free {Free plan} pro {Pro plan} team {Team plan} enterprise {Enterprise plan} other {Custom plan}}", "Plan name in the billing header."),
    ("{platform, select, ios {Open the iPhone app} android {Open the Android app} other {Open the web app}}", "Call to action under a QR code."),
    ("{device, select, mobile {Scan this code with your phone} desktop {Click the link below} other {Follow the instructions}}", "Two-factor setup hint."),
    ("{visibility, select, public {Anyone with the link can view} private {Only invited people can view} other {Visibility unknown}}", "Sharing dialog helper text."),
    ("{state, select, running {Running} queued {Queued} failed {Failed} success {Passed} other {Skipped}}", "CI job status."),
    ("{type, select, bug {Bug report} feature {Feature request} question {Question} other {Issue}}", "Issue type label."),
    ("{gender, select, female {Welcome back, {name}! She who returns gets a gift.} other {Welcome back, {name}!}}", "Greeting banner (playful copy)."),
    ("{reason, select, expired {Your session expired. Sign in again.} revoked {Your access was revoked by an admin.} other {You were signed out.}}", "Sign-out reason message."),
    ("{method, select, card {Pay by card} sepa {Pay by SEPA debit} paypal {Pay with PayPal} other {Choose a payment method}}", "Checkout payment method label."),
    ("{gender, select, female {Your friend {name} invited you. Join her on the team.} male {Your friend {name} invited you. Join him on the team.} other {Your friend {name} invited you. Join them on the team.}}", "Invitation email subject."),
    ("{tone, select, success {Saved} error {Could not save} other {Saving…}}", "Autosave indicator."),
    ("{priority, select, urgent {Urgent} high {High} medium {Medium} low {Low} other {None}}", "Priority selector option."),
    ("{theme, select, light {Light} dark {Dark} other {System}}", "Theme switch option."),
]
for src, d in SELECT:
    add("select", src, d, "label" if len(src) < 90 else "text")
for s, p, ctx in NOUNS[:12]:
    add("select", f"{{gender, select, female {{{{name}} shared a {s} with you}} male {{{{name}} shared a {s} with you}} other {{{{name}} shared a {s} with you}}}}",
        f"Notification in the {ctx}; gender of `name` (needed for agreement in some languages).", "toast")

# 3. selectordinal.
ORD = [
    ("You finished {rank, selectordinal, one {#st} two {#nd} few {#rd} other {#th}}", "Leaderboard result."),
    ("This is your {n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} visit this week", "Streak message."),
    ("{n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} attempt failed", "Login error after N attempts."),
    ("Renews on the {day, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} of each month", "Subscription renewal line."),
    ("Your {n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} anniversary with {company}", "Celebration banner."),
    ("{n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} place", "Podium label."),
    ("Reminder: the {n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} payment is due", "Billing reminder."),
    ("Step {n, selectordinal, one {#st} two {#nd} few {#rd} other {#th}} of the setup", "Wizard header."),
]
for src, d in ORD:
    add("selectordinal", src, d, "text")
for s, p, ctx in NOUNS[:11]:
    add("selectordinal", f"Your {{n, selectordinal, one {{#st}} two {{#nd}} few {{#rd}} other {{#th}}}} {s} this month",
        f"Milestone toast in the {ctx}.", "toast")

# 4. Nested select + plural, plural + tags.
for s, p, ctx in NOUNS[:20]:
    add("nested",
        f"{{gender, select, female {{{{name}} added {{count, plural, one {{# {s}}} other {{# {p}}}}} to her list}} male {{{{name}} added {{count, plural, one {{# {s}}} other {{# {p}}}}} to his list}} other {{{{name}} added {{count, plural, one {{# {s}}} other {{# {p}}}}} to their list}}}}",
        f"Activity line in the {ctx}: gender of the actor, then a count.", "text")
for s, p, ctx in NOUNS[:16]:
    add("nested",
        f"{{count, plural, =0 {{No {p} selected}} one {{<b>#</b> {s} selected. <link>Clear</link>}} other {{<b>#</b> {p} selected. <link>Clear</link>}}}}",
        f"Bulk-selection bar in the {ctx}.", "text")

# 5. Multiple placeholders (incl. ICU number/date formats).
MULTI = [
    "{actor} moved {item} from {from} to {to}",
    "{name} invited {email} to {team}",
    "{user} changed the due date of {task} to {date}",
    "Showing {start}–{end} of {total} results",
    "{name} commented on {title}: “{excerpt}”",
    "{author} merged {count, number} commits into {branch}",
    "Uploaded {fileName} ({size}) to {folder}",
    "{name} and {other} are typing…",
    "Signed in as {email} on {device}",
    "{percent, number, percent} of {quota} used",
    "Last synced {time} by {name}",
    "{user} renamed {oldName} to {newName}",
    "{used} of {limit} GB used — upgrade to get {extra} GB more",
    "{count, number} views since {date, date, medium}",
    "{name} assigned {task} to {assignee}",
    "{sender} sent you {amount, number, ::currency/EUR}",
    "Invoice {number} for {amount} is due on {date, date, long}",
    "{app} wants to access your {resource}",
    "From {start, time, short} to {end, time, short}",
    "{name} replied in {channel} at {time, time, short}",
    "Order {id} ships to {city}, {country}",
    "{owner}/{repo} · {stars} stars · updated {ago}",
    "Hello {firstName}, your trial ends in {days} days",
    "{author} wrote {words, number} words in {minutes} minutes",
    "{a} vs {b}: {delta, number, percent} difference",
    "{name} joined {team} via {method}",
    "Transfer {amount} from {source} to {target}?",
    "{count} of {total} checks passed on {branch}",
    "Rated {score} out of {max} by {reviewer}",
    "{city} · {temp}° · feels like {feels}°",
    "{name} ({role}) left {team}",
    "Moved {count} items to {folder} — {undo}",
    "Reset link sent to {email}. It expires in {minutes} minutes.",
    "{name} reacted {emoji} to your message in {channel}",
    "Your export of {rows, number} rows is ready: {link}",
    "{x} × {y} px, {size} KB",
    "Version {version} released on {date, date, short} by {author}",
    "Paid {amount} with card ending in {last4}",
    "{seconds, plural, one {# second} other {# seconds}} until {action}",
    "Between {min} and {max} characters, currently {count}",
]
for i, src in enumerate(MULTI):
    add("placeholders", src, "Sentence with several variables; order may change in the target language.", "text")
for s, p, ctx in NOUNS[:20]:
    add("placeholders", f"{{name}} deleted {{count, plural, one {{# {s}}} other {{# {p}}}}} from {{location}} on {{date, date, medium}}",
        f"Audit log entry in the {ctx}.", "text")

# 6. Rich tags.
TAGS = [
    "By continuing you accept the <terms>Terms</terms> and the <privacy>Privacy Policy</privacy>.",
    "<b>Heads up:</b> this action cannot be undone.",
    "Need help? <link>Contact support</link> or read the <docs>docs</docs>.",
    "Press <kbd>Enter</kbd> to send, <kbd>Shift</kbd>+<kbd>Enter</kbd> for a new line.",
    "Your trial ends in <b>{days} days</b>. <link>Upgrade now</link>",
    "Signed in as <b>{email}</b>. <link>Not you?</link>",
    "<b>{name}</b> mentioned you in <link>{title}</link>",
    "Drag files here or <link>browse</link>",
    "We sent a code to <b>{phone}</b>.<br/>It expires in 10 minutes.",
    "<i>Optional</i> — leave empty to use the default",
    "Read our <link>guide</link> to <b>get started</b> in minutes.",
    "This repository is <b>archived</b>. It is <i>read-only</i>.",
    "<b>Tip:</b> type <code>/</code> to open the command menu.",
    "Your card was declined. <link>Update payment details</link>",
    "<b>{count}</b> people viewed this <link>post</link> today",
    "Already have an account? <link>Sign in</link>",
    "Don't have an account? <link>Create one</link>",
    "Forgot your password? <link>Reset it</link>",
    "<strong>Warning:</strong> deleting <b>{name}</b> also deletes its data.",
    "Learn more about <link>keyboard shortcuts</link>",
    "Built with <heart>love</heart> by the community",
    "Reply to <b>{name}</b> in <channel>#{channel}</channel>",
    "Your changes are saved <b>automatically</b>.",
    "<b>New:</b> try the redesigned editor. <link>Learn more</link>",
    "Click <b>Publish</b> when you are ready.",
    "Use at least <b>8 characters</b> with a <b>number</b> and a <b>symbol</b>.",
    "Questions? Email us at <link>{email}</link>.",
    "<b>{percent}%</b> complete — <link>see details</link>",
    "You are viewing an <b>older version</b>. <link>Go to latest</link>",
    "Invite people with this link: <code>{url}</code>",
]
for t in TAGS:
    add("tags", t, "Rich text with markup tags rendered by the app.", "text")
for s, p, ctx in NOUNS[:20]:
    add("tags", f"<b>{{count, plural, one {{# {s}}} other {{# {p}}}}}</b> need your attention. <link>Review</link>",
        f"Callout in the {ctx}.", "text")

# 7. ICU apostrophes / escaping.
APOS = [
    "Don''t show this again", "It''s {name}''s turn", "You haven''t saved your changes",
    "Type '{'name'}' to insert the user''s name", "Use '{'curly braces'}' for variables",
    "We couldn''t find that page", "Can''t connect to the server", "{name}''s workspace",
    "That''s all for today!", "Let''s get started", "You''re all caught up",
    "Isn''t working? Try again", "Shouldn''t take long…", "The file ''{file}'' was renamed",
    "Here''s what''s new in {version}", "{count, plural, one {# person hasn''t replied} other {# people haven''t replied}}",
    "I''m not a robot", "What''s on your mind, {name}?", "Today''s schedule",
    "Write '#' to mention a channel", "{name}''s birthday is today", "Haven''t received the email?",
    "Wasn''t me — secure my account", "You''ll lose access on {date, date, long}", "Who''s online",
    "Aren''t you forgetting something?", "O''Brien & Partners invoice", "Here''s your receipt",
    "Didn''t get a code? Resend in {seconds}s", "We''ve updated our terms",
]
for a in APOS:
    add("apostrophe", a, "ICU apostrophe escaping: '' is a literal apostrophe, '{' quotes a brace.", "text")

# 8. Glossary terms.
GLOSS = [
    ("Create Workspace", "button"), ("Workspace settings", "heading"), ("Leave Workspace", "button"),
    ("Switch Workspace", "button"), ("Your Workspace is ready", "heading"),
    ("{name} invited you to the {workspace} Workspace", "text"), ("Delete this Workspace?", "heading"),
    ("Download Invoice", "button"), ("Invoice {number}", "heading"), ("Send Invoice to {email}", "button"),
    ("This Invoice is overdue", "text"), ("Void Invoice", "button"),
    ("New Project", "button"), ("Archive Project", "button"), ("Project settings", "heading"),
    ("{name} created the Project {project}", "text"), ("Rename Project", "button"),
    ("Back to Dashboard", "link"), ("Open Dashboard", "button"), ("Dashboard is loading…", "text"),
    ("Customize your Dashboard", "heading"),
    ("Billing", "heading"), ("Manage Billing", "button"), ("Billing email", "label"),
    ("Billing is handled by the Workspace owner", "text"),
    ("Each Project belongs to one Workspace", "text"), ("Add the Invoice to the Project", "button"),
    ("Pin this Project to your Dashboard", "button"), ("Billing history for this Workspace", "heading"),
    ("{count, plural, one {# Invoice} other {# Invoices}} in this Workspace", "label"),
    ("Moving a Project to another Workspace keeps its Invoice history", "text"),
    ("The Dashboard shows every Project you can access", "text"),
    ("Your Billing details appear on every Invoice", "text"),
]
for src, role in GLOSS:
    add("glossary", src, "Uses a glossary term with a mandatory translation.", role)

DNTS = [
    ("Sign in with GitHub", "button"), ("Connect Slack", "button"), ("Pay with Stripe", "button"),
    ("Powered by glot", "text"), ("Import from GitHub", "button"), ("Post updates to Slack", "label"),
    ("Your Stripe account is connected", "text"), ("Acme Cloud keeps your data in the EU", "text"),
    ("Sign in with OAuth", "button"), ("Create API key", "button"), ("API rate limit reached", "text"),
    ("Markdown is supported", "text"), ("Paste Markdown here", "label"), ("Revoke the API key {name}?", "heading"),
    ("glot translates your app on every pull request", "text"), ("Disconnect GitHub", "button"),
    ("{count, plural, one {# Slack channel} other {# Slack channels}} connected", "label"),
    ("Stripe will email the receipt to {email}", "text"), ("Acme uses cookies to keep you signed in", "text"),
    ("Export to Markdown", "button"), ("Your OAuth token expired", "text"), ("Read the API docs", "link"),
    ("Welcome to Acme, {name}!", "heading"), ("Sync issues with GitHub", "label"),
    ("Slack notifications are paused", "text"), ("Manage your Stripe subscription", "link"),
    ("glot found {count} untranslated keys", "text"), ("Copy the API URL", "button"),
    ("Install the Acme GitHub app", "button"), ("Send a test message to Slack", "button"),
]
for src, role in DNTS:
    add("dnt", src, "Contains a product/brand name that must stay verbatim.", role)

# 9. Short ambiguous strings with UI context (verb vs noun, domain).
AMBIG = [
    ("Save", "button", "Button in the editor toolbar: saves the document (verb)."),
    ("Close", "button", "Button closing a modal dialog (verb)."),
    ("Archive", "button", "Button moving an email to the archive (verb)."),
    ("Archive", "heading", "Title of the page listing archived emails (noun)."),
    ("Post", "button", "Publishes a forum message (verb)."),
    ("Post", "heading", "Header above a blog article (noun)."),
    ("Share", "button", "Opens the sharing dialog (verb)."),
    ("Order", "label", "Column header: sort order (noun, ordering)."),
    ("Order", "heading", "Purchase order detail page (noun, purchase)."),
    ("Free", "label", "Price tag: costs nothing."),
    ("Clear", "button", "Clears the search field (verb)."),
    ("Back", "button", "Navigates to the previous screen."),
    ("Run", "button", "Starts a CI pipeline (verb)."),
    ("Draft", "label", "Status badge: not published yet (noun/adjective)."),
    ("Match case", "label", "Checkbox in the find bar: case-sensitive search."),
    ("Apply", "button", "Applies the selected filters (verb)."),
    ("Book", "button", "Reserves a meeting room (verb)."),
    ("Track", "button", "Starts tracking a parcel (verb)."),
    ("Charge", "button", "Charges the customer's card (verb, payment)."),
    ("Charge", "label", "Battery level indicator (noun)."),
    ("Left", "label", "Remaining amount, as in '3 left' (not the direction)."),
    ("Second", "label", "Unit of time in a duration picker."),
    ("Second", "text", "Ordinal: second place in a ranking."),
    ("Bill", "heading", "Restaurant bill to pay (noun)."),
    ("Fine", "label", "Precision setting: fine vs coarse."),
    ("Mute", "button", "Mutes a conversation (verb)."),
    ("Pin", "button", "Pins a message to the top (verb)."),
    ("Pin", "label", "Field asking for the card PIN (noun)."),
    ("Follow", "button", "Follows a user profile (verb)."),
    ("Feed", "heading", "Title of the activity feed (noun)."),
    ("Kind", "label", "Column header: type/category of resource (noun)."),
    ("Present", "button", "Starts a slideshow presentation (verb)."),
    ("Present", "label", "Attendance status: the person is here."),
    ("Export", "button", "Exports the table as CSV (verb)."),
    ("Lead", "label", "Sales CRM: a potential customer (noun)."),
    ("Board", "heading", "Kanban board title (noun)."),
    ("Ship", "button", "Marks an order as shipped (verb)."),
    ("Report", "button", "Reports abusive content (verb)."),
    ("Report", "heading", "Title of the analytics report (noun)."),
    ("Close", "label", "Stock market: closing price (noun)."),
]
for src, role, d in AMBIG:
    seen.discard(src)  # same text, different context: intentionally kept
    add("ambiguous", src, d, role)

# 10. Width-constrained buttons / tabs (Inter 14 px, 96 px).
WIDTH = [
    "Save changes", "Sign up free", "Start trial", "Upgrade", "Invite team", "Add member", "Log out",
    "Try again", "Get started", "Learn more", "Continue", "Cancel", "Confirm", "Download", "Delete",
    "Resend code", "Show more", "Mark as read", "Go back", "Subscribe", "Unsubscribe", "Book a demo",
    "Copy link", "Edit profile", "New folder", "Select all", "Sort by date", "View all", "Retry",
    "Accept", "Decline", "Submit", "Next step", "Skip", "Finish",
]
for i, w in enumerate(WIDTH):
    add("width", w, "Primary button with a fixed width: the label must fit on one line.", "button",
        BUTTON_96 if i % 3 else BUTTON_120)
for w in ["Overview", "Settings", "Activity", "Members", "Security", "Integrations", "Usage", "Logs"]:
    add("width", w, "Tab label in a narrow tab bar.", "tab", TAB_80)
for w in ["Your plan renews automatically every month.", "Invite teammates to collaborate in real time.",
          "Connect your bank to get paid faster."]:
    add("width", w, "Two-line card description with a fixed width.", "text", CARD_2L)

# 11. Dates / numbers / currency formats.
DATES = [
    "Updated {date, date, short}", "Due {date, date, full}", "Starts at {time, time, short}",
    "{amount, number, ::currency/USD} per month", "{n, number, integer} people", "{ratio, number, percent} complete",
    "Joined {date, date, ::yyyyMMMM}", "Total: {total, number, ::currency/EUR}", "{size, number, ::compact-short} downloads",
    "{bytes, number, ::unit/megabyte} free", "Next charge on {date, date, medium} at {time, time, short}",
    "{count, plural, one {Billed yearly: {price, number, ::currency/USD}} other {Billed every # years: {price, number, ::currency/USD}}}",
    "{temp, number, ::unit/celsius} outside", "Average: {avg, number, ::precision-integer} ms",
    "{distance, number, ::unit/kilometer} away", "Expires {date, date, long}", "Created {date, date} at {time, time}",
    "Discount: {off, number, percent}", "{rating, number, ::precision-integer} stars", "Balance: {balance, number, ::currency/GBP}",
]
for d in DATES:
    add("datenum", d, "Formatted number/date argument; keep the format and its skeleton verbatim.", "label")

assert len(out) == 500, len(out)
with OUT.open("w") as f:
    for m in out:
        f.write(json.dumps(m, ensure_ascii=False) + "\n")
cats = {}
for m in out:
    cats[m["category"]] = cats.get(m["category"], 0) + 1
print(len(out), cats)
