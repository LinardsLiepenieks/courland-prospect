// A full-screen "Drafting reply…" overlay shown in a background draft tab while
// it opens its assigned conversation and the AI reply is generated. Purely a
// status surface: it masks LinkedIn's list→thread churn and blocks stray clicks
// on a tab that's mid-work, then lifts the moment the draft starts typing in.
// Self-contained (mirrors toast.ts); failures never propagate into the page.

import "./overlay.css";

const OVERLAY_ID = "cp-draft-overlay";

/** Show the full-screen "Drafting reply…" overlay. Idempotent — a second call
 *  while it's up is a no-op. Enters on the next frames (from the CSS hidden
 *  state) so the fade has something to animate from. */
export function showDraftOverlay(): void {
  try {
    if (document.getElementById(OVERLAY_ID)) return;
    const host = document.body ?? document.documentElement;

    const overlay = document.createElement("div");
    overlay.id = OVERLAY_ID;
    overlay.className = "cp-draft-overlay";
    overlay.setAttribute("role", "status");
    overlay.setAttribute("aria-live", "polite");
    overlay.setAttribute("aria-label", "Drafting reply");

    const card = document.createElement("div");
    card.className = "cp-draft-overlay__card";
    const spinner = document.createElement("div");
    spinner.className = "cp-draft-overlay__spinner";
    const label = document.createElement("div");
    label.className = "cp-draft-overlay__label";
    label.textContent = "Drafting reply…";
    card.append(spinner, label);
    overlay.append(card);
    host.append(overlay);

    // Two frames so it paints in the hidden state before we flip to enter —
    // otherwise the transition has nothing to animate from.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        overlay.dataset.enter = "true";
      }),
    );
  } catch {
    // Never throw into the LinkedIn page.
  }
}

/** Remove the overlay. Idempotent. Fades out then removes; a timeout backstops
 *  the case where transitionend never fires (reduced motion / detached node). */
export function hideDraftOverlay(): void {
  try {
    const overlay = document.getElementById(OVERLAY_ID);
    if (!overlay) return;
    overlay.dataset.enter = "false"; // back to the hidden state
    const remove = () => overlay.remove();
    overlay.addEventListener("transitionend", remove, { once: true });
    window.setTimeout(remove, 320);
  } catch {
    // Never throw into the LinkedIn page.
  }
}
