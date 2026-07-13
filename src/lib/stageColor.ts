import type { CSSProperties } from "react";
import type { StageColor } from "../api/stages";

/** Inline style exposing a stage's accent as the `--stage-accent` custom
 *  property, so CSS modules can theme borders, dots, and (via color-mix) soft
 *  tints from one variable. The token maps to the themed `--stage-<token>`
 *  variables defined in global.css. Falls back to `--stage-gray` for any token
 *  without a matching variable, so an unexpected value (Rust types `color` as a
 *  plain String) degrades to a neutral accent instead of breaking every
 *  `color-mix` that reads `--stage-accent`. */
export function stageAccentStyle(color: StageColor): CSSProperties {
  return { "--stage-accent": `var(--stage-${color}, var(--stage-gray))` } as CSSProperties;
}
