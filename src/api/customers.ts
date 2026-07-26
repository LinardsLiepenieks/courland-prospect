import { invoke } from "@tauri-apps/api/core";

/**
 * A customer profile (ICP): one kind of buyer for the single product.
 *
 * The three text fields are what steers a draft, and each does a distinct job —
 * a goal alone tells the AI the destination but not the terrain, so it can't
 * choose between two snippets.
 */
export interface Customer {
  id: number;
  name: string;
  /** How you recognize one: role, company shape, where you find them. */
  who_they_are: string;
  /** What they care about / what's broken for them today. */
  pain: string;
  /** What a thread with them should achieve. Free prose, not a pipeline stage:
   *  the pipeline is your process and is shared by every customer, while the
   *  goal is intent. */
  goal: string;
  created_at: string;
}

// Typed wrappers over the Rust commands. All SQL lives in the backend; these
// are the only entry points the UI uses to touch customer data.

export function listCustomers(): Promise<Customer[]> {
  return invoke("list_customers");
}

export function createCustomer(
  name: string,
  whoTheyAre: string,
  pain: string,
  goal: string,
): Promise<Customer> {
  return invoke("create_customer", { name, whoTheyAre, pain, goal });
}

export function updateCustomer(
  id: number,
  name: string,
  whoTheyAre: string,
  pain: string,
  goal: string,
): Promise<Customer> {
  return invoke("update_customer", { id, name, whoTheyAre, pain, goal });
}

/** Delete a customer profile. Its prospects are NOT deleted — they stay in the
 *  pipeline and simply become unassigned, so nothing captured is lost. */
export function deleteCustomer(id: number): Promise<void> {
  return invoke("delete_customer", { id });
}
