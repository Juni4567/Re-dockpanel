import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

/**
 * The panel's confirmation prompt.
 *
 * ── Why this exists ─────────────────────────────────────────────────────────
 * Issue #120: clicking "nginx" to switch the reverse proxy back "appears to do
 * nothing at all". It did arm a confirmation — the bar just rendered at the top
 * of Settings while the control that armed it sits near the bottom, so on any
 * normal viewport the operator was looking at an unchanged screen. He concluded
 * the panel was refusing the switch, and found the Confirm button by reading our
 * source.
 *
 * That was not one misplaced div. A sweep of the SPA found 54 armed-confirmation
 * states: 31 swap Confirm/Cancel into the trigger's own place and are fine, and
 * 23 are detached banners rendered high in the subtree with their triggers
 * scattered below. NONE of the 23 carried `sticky`, `scrollIntoView`,
 * `role="alert"`, `aria-live` or `autoFocus`, and there is no toast system in
 * this SPA — so arming produced no scroll, no focus and no announcement, only a
 * layout shift that pushed the button DOWN, out from under the cursor that had
 * just clicked it. The instances cluster because a correct one was awkward to
 * write, not because thirteen authors were careless.
 *
 * ── Why a portal, and not a sticky bar ──────────────────────────────────────
 * The obvious repair — make the bar `sticky top-20` — is worse than it looks.
 * Every one of those bars is `bg-*-500/5`: five percent alpha over nothing. The
 * page header only survives being sticky because index.css paints it an opaque
 * `--color-dark-900`. Sticking a transparent bar makes the prompt VISIBLE and
 * ILLEGIBLE, with the page scrolling through the text — and a reviewer checking
 * "did something appear?" would pass it. The offset is unfixable too: the page
 * header wraps at tablet width, and GlassLayout and CommandLayout each render a
 * `sticky top-0` mobile header INSIDE the same `<main>` scroll container, so the
 * correct offset differs by layout, and the operator picks the layout at runtime.
 *
 * Rendering into `document.body` sidesteps all of it: no offset to get wrong, no
 * stacking context to lose, no scroll container to be trapped in, and it cannot
 * land on a tab the operator has since navigated away from.
 *
 * ── Two deliberate details ──────────────────────────────────────────────────
 *  - Focus goes to CANCEL, never to Confirm. Auto-focusing the confirm button
 *    would leave a destructive action — uninstall Traefik, revoke every session,
 *    purge an entire Cloudflare zone — one Enter keypress away from an operator
 *    who has not read the sentence yet.
 *  - `role="alertdialog"` with `aria-modal`, because this genuinely is modal:
 *    the overlay blocks the page. That is the one ARIA role that matches, and it
 *    only became true once the prompt stopped being a bar you could scroll away
 *    from.
 */
export default function ConfirmDialog({
  title,
  label,
  consequences,
  confirmLabel = "Confirm",
  tone = "danger",
  onConfirm,
  onCancel,
}: {
  title?: string;
  label: string;
  consequences?: string[];
  confirmLabel?: string;
  tone?: "danger" | "warn";
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return createPortal(
    <div
      className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4 dp-modal-overlay animate-fade-in"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div
        role="alertdialog"
        aria-modal="true"
        aria-label={title || label}
        className="bg-dark-800 rounded-xl shadow-2xl p-6 w-full max-w-lg border border-dark-600 dp-modal animate-scale-in"
      >
        <div className="flex items-start gap-3.5 mb-4">
          <div className={`p-2.5 rounded-lg shrink-0 ${
            tone === "warn"
              ? "bg-warn-500/15 text-warn-400 border border-warn-500/30"
              : "bg-danger-500/15 text-danger-400 border border-danger-500/30"
          }`}>
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
            </svg>
          </div>
          <div>
            <h3 className="text-base font-bold text-white tracking-tight">
              {title || (tone === "danger" ? "Confirm Lethal Action" : "Confirm Action")}
            </h3>
            <p className="text-xs text-dark-300 mt-1 leading-relaxed whitespace-pre-line">{label}</p>
          </div>
        </div>

        {consequences && consequences.length > 0 && (
          <div className="mb-5 bg-dark-900/90 border border-dark-700/80 rounded-lg p-4">
            <h4 className="text-[11px] font-bold text-dark-300 uppercase tracking-wider mb-2.5 flex items-center gap-1.5 font-mono">
              <span className="w-1.5 h-1.5 rounded-full bg-danger-400 animate-pulse"></span>
              Consequences & System Impact:
            </h4>
            <ul className="space-y-1.5 text-xs text-dark-200">
              {consequences.map((c, i) => (
                <li key={i} className="flex items-start gap-2">
                  <span className="text-danger-400 font-bold mt-0.5">•</span>
                  <span className="leading-snug">{c}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="flex justify-end gap-2.5 pt-2 border-t border-dark-700/50">
          <button
            ref={cancelRef}
            onClick={onCancel}
            className="px-4 py-2 bg-dark-700 text-dark-200 text-xs font-semibold rounded-lg hover:bg-dark-600 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className={`px-4 py-2 text-white text-xs font-semibold rounded-lg shadow-sm transition-colors ${
              tone === "warn"
                ? "bg-warn-500 hover:bg-warn-600"
                : "bg-danger-500 hover:bg-danger-600"
            }`}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
