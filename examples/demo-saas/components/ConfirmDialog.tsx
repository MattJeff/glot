"use client";

export function ConfirmDialog(props: {
  title: string;
  body: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div role="dialog" aria-modal="true" aria-labelledby="confirm-title" className="fixed inset-0 z-20 grid place-items-center bg-slate-900/40">
      <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl">
        <h2 id="confirm-title" className="text-lg font-semibold">{props.title}</h2>
        <p className="mt-2 text-sm text-slate-600">{props.body}</p>
        <div className="mt-6 flex justify-end gap-2">
          <button onClick={props.onCancel} className="rounded-md px-3 py-2 text-sm text-slate-600 hover:bg-slate-100">
            Cancel
          </button>
          <button onClick={props.onConfirm} className="rounded-md bg-rose-600 px-3 py-2 text-sm font-medium text-white">
            {props.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
