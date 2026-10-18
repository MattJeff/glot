import { PricingTable } from "@/components/PricingTable";

const FAQ = [
  { q: "Can I change plans later?", a: "Yes. Upgrades apply immediately and downgrades at the end of your billing period." },
  { q: "Do you charge per invoice?", a: "Never. Every plan has a flat monthly price." },
  { q: "Is there a free trial?", a: "Growth and Scale come with a 14-day free trial, no credit card required." },
];

export default function PricingPage() {
  return (
    <div>
      <div className="text-center">
        <h1 className="text-4xl font-bold tracking-tight">Simple pricing for teams of every size</h1>
        <p className="mt-3 text-slate-500">Send invoices, get paid faster. Cancel anytime.</p>
      </div>
      <div className="mt-8">
        <PricingTable />
      </div>
      <section className="mx-auto mt-16 max-w-2xl">
        <h2 className="text-xl font-semibold">Frequently asked questions</h2>
        <dl className="mt-6 space-y-6">
          {FAQ.map((f) => (
            <div key={f.q}>
              <dt className="font-medium">{f.q}</dt>
              <dd className="mt-1 text-sm text-slate-500">{f.a}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
