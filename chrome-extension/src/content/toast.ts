// Small, quiet capture-feedback toasts injected top-right on the LinkedIn page.
// This is the extension's only page-level surface besides the compose widget, so
// it's self-contained: one `showToast(kind, message)` call, no setup. Failures
// never propagate into the host page.

import "./toast.css";

export type ToastKind = "success" | "info" | "error";

const CONTAINER_ID = "cp-toaster";

/** Auto-dismiss timing per kind — success is a quick confirmation; info/error
 *  linger longer since they carry something the user may want to read. */
const DURATION_MS: Record<ToastKind, number> = {
  success: 2600,
  info: 4200,
  error: 4600,
};

function toaster(): HTMLElement {
  let c = document.getElementById(CONTAINER_ID);
  if (!c) {
    c = document.createElement("div");
    c.id = CONTAINER_ID;
    c.className = "cp-toaster";
    (document.body ?? document.documentElement).append(c);
  }
  return c;
}

/** Show a transient toast. Enters on the next frame (from the CSS hidden state),
 *  auto-dismisses after {@link DURATION_MS}, and can be clicked to dismiss early.
 *  Uses transitions (not keyframes) so rapidly-stacked toasts retarget smoothly. */
export function showToast(kind: ToastKind, message: string): void {
  try {
    const toast = document.createElement("div");
    toast.className = "cp-toast";
    toast.dataset.kind = kind;
    // Errors are assertive so a screen reader announces them; the rest are polite.
    toast.setAttribute("role", kind === "error" ? "alert" : "status");

    const dot = document.createElement("span");
    dot.className = "cp-toast__dot";
    const text = document.createElement("span");
    text.className = "cp-toast__text";
    text.textContent = message;
    toast.append(dot, text);

    toaster().append(toast);

    // Two frames so the element paints in its hidden state before we flip to the
    // enter state — otherwise the transition has nothing to animate from.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        toast.dataset.enter = "true";
      }),
    );

    let done = false;
    function dismiss(): void {
      if (done) return;
      done = true;
      toast.dataset.enter = "false"; // back to the hidden/exit state
      const remove = () => toast.remove();
      toast.addEventListener("transitionend", remove, { once: true });
      // Backstop if transitionend never fires (reduced motion, detached node).
      window.setTimeout(remove, 260);
    }

    window.setTimeout(dismiss, DURATION_MS[kind]);
    toast.addEventListener("click", dismiss);
  } catch {
    // Never throw into the LinkedIn page.
  }
}
