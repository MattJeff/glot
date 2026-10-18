import { team } from "@/lib/data";
import { formatDate } from "@/lib/format";
import { SettingsTabs } from "@/components/SettingsTabs";
import { InviteMemberForm } from "@/components/InviteMemberForm";

const SEATS = 5;

export default function TeamSettingsPage() {
  return (
    <div>
      <SettingsTabs active="team" />
      <section className="space-y-6 rounded-xl border border-slate-200 bg-white p-6">
        <div>
          <h2 className="font-semibold">Team members</h2>
          <p className="text-sm text-slate-500">
            {`${team.length} of ${SEATS} seats used. Members can create and send invoices; admins can also manage billing.`}
          </p>
        </div>
        <InviteMemberForm seatsLeft={SEATS - team.length} />
        <ul className="divide-y divide-slate-100">
          {team.map((m) => (
            <li key={m.email} className="flex items-center justify-between py-3 text-sm">
              <div>
                <p className="font-medium">{m.name}</p>
                <p className="text-slate-500">{m.email}</p>
              </div>
              <div className="flex items-center gap-6">
                <span className="text-slate-500">Last active {formatDate(m.lastActive)}</span>
                <span className="w-20 rounded-full bg-slate-100 px-2 py-0.5 text-center text-xs">{m.role}</span>
                {m.role !== "Owner" && (
                  <button aria-label={`Remove ${m.name} from the team`} className="text-rose-600 hover:underline">
                    Remove
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      </section>
      <section className="mt-6 rounded-xl border border-rose-200 bg-white p-6">
        <h2 className="font-semibold text-rose-700">Danger zone</h2>
        <p className="mt-1 text-sm text-slate-500">Deleting the workspace removes all invoices and customers. This cannot be undone.</p>
        <button className="mt-4 w-40 rounded-md border border-rose-300 py-2 text-sm font-medium text-rose-700">Delete workspace</button>
      </section>
    </div>
  );
}
