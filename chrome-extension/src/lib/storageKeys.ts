/** `chrome.storage.local` keys shared across content surfaces. Centralized so
 *  the capture widget and the "Draft for" control can't drift apart on the key
 *  and silently stop sharing the user's last-picked pitch. */
export const LAST_PITCH_KEY = "lastPitchId";
