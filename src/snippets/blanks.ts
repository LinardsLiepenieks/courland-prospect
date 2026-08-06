/**
 * Splitting snippet content on its blanks — the `[first name]` slots that get
 * filled in when a message is actually drafted.
 *
 * A blank stands in for the one detail that was welded to a single person, so the
 * rest of the line can be reused. The backend writes them (see the propose pass) and
 * the draft composer fills them; this is only about showing them, so it is
 * deliberately more forgiving than the backend's parser: anything that isn't a
 * cleanly-closed, non-empty `[label]` is rendered as ordinary text. A view must
 * never silently swallow characters the user wrote.
 */

export type ContentPart =
  | { kind: "text"; value: string }
  | { kind: "blank"; label: string };

/** Split `content` into its literal text and its blanks, in order. */
export function splitOnBlanks(content: string): ContentPart[] {
  const parts: ContentPart[] = [];
  let text = "";

  for (let i = 0; i < content.length; ) {
    if (content[i] === "[") {
      const close = content.indexOf("]", i + 1);
      const label = close === -1 ? "" : content.slice(i + 1, close).trim();
      if (label) {
        if (text) {
          parts.push({ kind: "text", value: text });
          text = "";
        }
        parts.push({ kind: "blank", label });
        i = close + 1;
        continue;
      }
    }
    text += content[i];
    i += 1;
  }

  if (text) parts.push({ kind: "text", value: text });
  return parts;
}

/** Whether `content` carries at least one blank. */
export function hasBlank(content: string): boolean {
  return splitOnBlanks(content).some((part) => part.kind === "blank");
}
