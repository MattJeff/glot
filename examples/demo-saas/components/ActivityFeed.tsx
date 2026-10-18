const ACTIVITY = [
  { who: "Grace Hopper", what: "sent invoice INV-1046 to Wayne Enterprises", when: "2 hours ago" },
  { who: "Initech", what: "paid invoice INV-1043", when: "Yesterday" },
  { who: "Ada Lovelace", what: "invited Alan Turing to the team", when: "3 days ago" },
];

export function ActivityFeed() {
  return (
    <section className="rounded-xl border border-slate-200 bg-white p-5">
      <h2 className="font-semibold">Recent activity</h2>
      <ul className="mt-4 space-y-3 text-sm">
        {ACTIVITY.map((a, i) => (
          <li key={i} className="flex justify-between gap-4">
            <span>
              <strong>{a.who}</strong> {a.what}
            </span>
            <span className="shrink-0 text-slate-400">{a.when}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
