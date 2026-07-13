/** `chrome.storage.local` keys shared across content surfaces. Centralized so
 *  the capture widget and the "Draft for" control can't drift apart on the key
 *  and silently stop sharing the user's last-picked pitch. */
export const LAST_PITCH_KEY = "lastPitchId";

/** URL-hash marker the review-tab queue appends to a thread URL, and the content
 *  script keys off to know a tab was opened to be pre-filled with a cached draft
 *  (vs. a thread the user opened by hand). Shared so the SW that builds the URL
 *  and the content script that detects it can't drift apart. */
export const FILL_HASH = "cpfill";
