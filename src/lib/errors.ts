/** Normalize an unknown thrown value into a display string. Tauri command
 *  rejections come through as strings; JS errors carry `.message`. */
export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
