export type InvoiceStatus = "paid" | "pending" | "overdue" | "draft";

export type Invoice = {
  id: string;
  number: string;
  customer: string;
  email: string;
  amount: number;
  currency: string;
  issuedAt: string;
  dueAt: string;
  status: InvoiceStatus;
  items: { description: string; quantity: number; unitPrice: number }[];
};

export const invoices: Invoice[] = [
  {
    id: "inv_1042",
    number: "INV-1042",
    customer: "Globex Corporation",
    email: "billing@globex.com",
    amount: 4250,
    currency: "USD",
    issuedAt: "2026-09-02",
    dueAt: "2026-10-02",
    status: "overdue",
    items: [
      { description: "Website redesign — phase 2", quantity: 1, unitPrice: 3500 },
      { description: "Hosting (3 months)", quantity: 3, unitPrice: 250 },
    ],
  },
  {
    id: "inv_1043",
    number: "INV-1043",
    customer: "Initech",
    email: "accounts@initech.io",
    amount: 1200,
    currency: "USD",
    issuedAt: "2026-09-15",
    dueAt: "2026-10-15",
    status: "paid",
    items: [{ description: "Consulting hours", quantity: 8, unitPrice: 150 }],
  },
  {
    id: "inv_1044",
    number: "INV-1044",
    customer: "Umbrella Health",
    email: "finance@umbrella.health",
    amount: 9800,
    currency: "EUR",
    issuedAt: "2026-10-01",
    dueAt: "2026-10-31",
    status: "pending",
    items: [
      { description: "Annual support plan", quantity: 1, unitPrice: 8000 },
      { description: "Onboarding workshop", quantity: 2, unitPrice: 900 },
    ],
  },
  {
    id: "inv_1045",
    number: "INV-1045",
    customer: "Stark Industries",
    email: "ap@stark.com",
    amount: 640,
    currency: "USD",
    issuedAt: "2026-10-10",
    dueAt: "2026-11-09",
    status: "draft",
    items: [{ description: "Design assets license", quantity: 4, unitPrice: 160 }],
  },
  {
    id: "inv_1046",
    number: "INV-1046",
    customer: "Wayne Enterprises",
    email: "treasury@wayne.com",
    amount: 15400,
    currency: "USD",
    issuedAt: "2026-10-12",
    dueAt: "2026-11-11",
    status: "pending",
    items: [{ description: "Security audit", quantity: 1, unitPrice: 15400 }],
  },
];

export const team = [
  { name: "Ada Lovelace", email: "ada@acme.dev", role: "Owner", lastActive: "2026-10-18" },
  { name: "Grace Hopper", email: "grace@acme.dev", role: "Admin", lastActive: "2026-10-16" },
  { name: "Alan Turing", email: "alan@acme.dev", role: "Member", lastActive: "2026-09-30" },
];

// Trap: user-facing copy living in config objects, not JSX.
export const STATUS_META: Record<InvoiceStatus, { label: string; hint: string; className: string }> = {
  paid: { label: "Paid", hint: "Payment received in full", className: "bg-emerald-50 text-emerald-700 ring-emerald-600/20" },
  pending: { label: "Pending", hint: "Waiting for the customer to pay", className: "bg-amber-50 text-amber-700 ring-amber-600/20" },
  overdue: { label: "Overdue", hint: "The due date has passed", className: "bg-rose-50 text-rose-700 ring-rose-600/20" },
  draft: { label: "Draft", hint: "Not sent to the customer yet", className: "bg-slate-100 text-slate-600 ring-slate-500/20" },
};

export const NAV = [
  { href: "/", label: "Dashboard", icon: "◧" },
  { href: "/invoices", label: "Invoices", icon: "▤" },
  { href: "/settings/team", label: "Team", icon: "☺" },
  { href: "/settings/billing", label: "Billing", icon: "$" },
  { href: "/pricing", label: "Pricing", icon: "★" },
];

export const PLANS = [
  {
    name: "Starter",
    price: 0,
    tagline: "For freelancers sending their first invoices.",
    features: ["Up to 5 invoices per month", "1 team member", "Email support"],
    cta: "Start for free",
  },
  {
    name: "Growth",
    price: 29,
    tagline: "For small teams that get paid on time.",
    features: ["Unlimited invoices", "Up to 10 team members", "Automatic payment reminders", "Custom branding"],
    cta: "Upgrade now",
    highlighted: true,
  },
  {
    name: "Scale",
    price: 99,
    tagline: "For finance teams with serious volume.",
    features: ["Everything in Growth", "Unlimited team members", "SSO & audit log", "Priority support"],
    cta: "Contact sales",
  },
];

export const ONBOARDING_STEPS = [
  { title: "Add your company details", description: "Your logo and address appear on every invoice." },
  { title: "Connect a payment method", description: "Let customers pay by card or bank transfer." },
  { title: "Send your first invoice", description: "It takes less than a minute." },
];
