export function formatMoney(amount: number, currency = "USD") {
  return new Intl.NumberFormat("en-US", { style: "currency", currency }).format(amount);
}

export function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}

// Trap: English-only pluralization by string concatenation.
export function pluralize(n: number, word: string) {
  return n + " " + word + (n === 1 ? "" : "s");
}

export function daysUntil(iso: string) {
  const diff = Math.round((new Date(iso).getTime() - new Date("2026-10-18").getTime()) / 86400000);
  if (diff === 0) return "Due today";
  if (diff < 0) return `${Math.abs(diff)} days overdue`;
  return `Due in ${diff} days`;
}
