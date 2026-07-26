import { invoke } from "@tauri-apps/api/core";

/** The one product being sold — the invariant half of every message. A
 *  singleton: exactly one record exists, so there's no create or delete. */
export interface Product {
  name: string;
  /** The long-form product story: what it is, who it's for, how it works, why
   *  it beats the alternative. The AI's single source of product truth. */
  description: string;
  updated_at: string;
}

// Typed wrappers over the Rust commands. All SQL lives in the backend.

export function getProduct(): Promise<Product> {
  return invoke("get_product");
}

export function updateProduct(
  name: string,
  description: string,
): Promise<Product> {
  return invoke("update_product", { name, description });
}

/** Polish the product description through the local Claude Code CLI, returning
 *  the rewritten version. Doesn't persist — the caller drops the result into the
 *  editor for the user to review and save. */
export function polishProduct(text: string): Promise<string> {
  return invoke("polish_product", { text });
}
