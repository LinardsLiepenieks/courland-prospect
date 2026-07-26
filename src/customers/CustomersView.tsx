import { useCallback, useRef, useState } from "react";
import {
  createCustomer,
  deleteCustomer,
  listCustomers,
  updateCustomer,
  type Customer,
} from "../api/customers";
import LoadError from "../components/LoadError";
import SavedIndicator from "../components/SavedIndicator";
import { errorMessage } from "../lib/errors";
import { useAutosave } from "../lib/useAutosave";
import { useLoad } from "../lib/useLoad";
import styles from "./CustomersView.module.css";

/** Customers tab: the kinds of buyer you sell the one product to.
 *
 *  Each profile is pure steering — it owns no pipeline and no snippets. What it
 *  carries is what the AI can't infer: who this person is, what hurts, and where
 *  a thread with them should land. */
export default function CustomersView() {
  const {
    data: customers,
    error: loadError,
    setData: setCustomers,
    retry,
  } = useLoad(listCustomers);
  const [actionError, setActionError] = useState<string | null>(null);
  // The card showing its full editor. Only one at a time — the collapsed rows
  // stay scannable, and a newly added profile opens straight into editing.
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [adding, setAdding] = useState(false);
  // Synchronous re-entry guard (state updates are async).
  const addingRef = useRef(false);

  const handleAdd = useCallback(async () => {
    if (addingRef.current) return;
    addingRef.current = true;
    setAdding(true);
    setActionError(null);
    try {
      const created = await createCustomer("New customer", "", "", "");
      setCustomers((prev) => (prev ? [...prev, created] : [created]));
      setExpandedId(created.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      addingRef.current = false;
      setAdding(false);
    }
  }, []);

  // Throws on failure so the card can surface the error and stay put.
  const handleDelete = useCallback(async (id: number) => {
    await deleteCustomer(id);
    setCustomers((prev) => prev && prev.filter((c) => c.id !== id));
  }, []);

  // Keeps the collapsed summary in step with what the card's autosave persisted.
  const handleSaved = useCallback((saved: Customer) => {
    setCustomers((prev) => prev && prev.map((c) => (c.id === saved.id ? saved : c)));
  }, []);

  if (loadError) {
    return (
      <LoadError what="your customer profiles" detail={loadError} onRetry={retry} />
    );
  }

  return (
    <div className={styles.customers}>
      <header className={styles.intro}>
        <h1 className={styles.title}>Customers</h1>
        <p className={styles.subtitle}>
          The kinds of buyer you sell to. The product never changes — these do,
          and they're what steer a draft toward the right snippets and the right
          ask.
        </p>
      </header>

      {actionError && (
        <div className={styles.error} role="alert">
          {actionError}
        </div>
      )}

      {!customers ? (
        // Local SQLite resolves near-instantly; reserve layout without a
        // spinner flash on the common fast path.
        <div className={styles.loading} aria-busy="true" aria-hidden="true" />
      ) : (
        <>
          {customers.length === 0 ? (
            <div className={styles.blank}>
              <h2 className={styles.blankTitle}>No customer profiles yet</h2>
              <p className={styles.blankBody}>
                Add one for each kind of buyer you talk to. Prospects without a
                profile still sit in your pipeline — their drafts just won't have
                a goal to aim at.
              </p>
            </div>
          ) : (
            <ul className={styles.list}>
              {customers.map((c) => (
                <CustomerCard
                  key={c.id}
                  customer={c}
                  expanded={expandedId === c.id}
                  onToggle={() =>
                    setExpandedId((cur) => (cur === c.id ? null : c.id))
                  }
                  onSaved={handleSaved}
                  onDelete={() => handleDelete(c.id)}
                />
              ))}
            </ul>
          )}

          <button
            type="button"
            className={styles.addBtn}
            onClick={() => void handleAdd()}
            disabled={adding}
          >
            <PlusIcon />
            {adding ? "Adding…" : "Add customer profile"}
          </button>
        </>
      )}
    </div>
  );
}

interface CardProps {
  customer: Customer;
  expanded: boolean;
  onToggle: () => void;
  onSaved: (saved: Customer) => void;
  onDelete: () => Promise<void>;
}

/** One customer profile: a summary row that expands into its editor. */
function CustomerCard({ customer, expanded, onToggle, onSaved, onDelete }: CardProps) {
  return (
    <li className={styles.card} data-expanded={expanded || undefined}>
      <button
        type="button"
        className={styles.cardHead}
        onClick={onToggle}
        aria-expanded={expanded}
      >
        <Chevron expanded={expanded} />
        <span className={styles.cardName}>{customer.name}</span>
        <span className={styles.cardGoal}>
          {customer.goal.trim() ? (
            <>
              <GoalIcon />
              {customer.goal}
            </>
          ) : (
            <span className={styles.cardGoalEmpty}>No goal set</span>
          )}
        </span>
      </button>

      {/* Unmounted when collapsed rather than hidden: the editor autosaves on
          unmount, so collapsing a card flushes any edit made in the last beat
          instead of stranding it. Keyed by id so a card always mounts with the
          right baseline. */}
      {expanded && (
        <CustomerForm
          key={customer.id}
          customer={customer}
          onSaved={onSaved}
          onDelete={onDelete}
        />
      )}
    </li>
  );
}

function CustomerForm({
  customer,
  onSaved,
  onDelete,
}: {
  customer: Customer;
  onSaved: (saved: Customer) => void;
  onDelete: () => Promise<void>;
}) {
  const [name, setName] = useState(customer.name);
  const [whoTheyAre, setWhoTheyAre] = useState(customer.who_they_are);
  const [pain, setPain] = useState(customer.pain);
  const [goal, setGoal] = useState(customer.goal);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Synchronous re-entry guards (state updates are async).
  const deletingRef = useRef(false);
  // Once deletion starts the row is going away — the unmount flush must not
  // resurrect it by writing to a deleted id.
  const deletedRef = useRef(false);

  const { saving, showSaved, error } = useAutosave({
    values: { name, whoTheyAre, pain, goal },
    persist: async (v) => {
      // A profile is identified by its name everywhere it's offered — the board's
      // steering menu, the extension's capture picker — so a blank one isn't
      // saveable. Refuse it out loud. Quietly substituting the stored name was
      // worse than an inline error: autosave would record the BLANK as the saved
      // baseline and flash "Saved", so the collapsed header kept a name the open
      // field said was gone, and the change the user actually made to the other
      // fields went in under a name they thought they'd cleared.
      if (!v.name) throw new Error("A customer profile needs a name.");
      const saved = await updateCustomer(
        customer.id,
        v.name,
        v.whoTheyAre,
        v.pain,
        v.goal,
      );
      onSaved(saved);
    },
    canFlush: () => !deletedRef.current,
  });

  async function handleDelete() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    deletedRef.current = true;
    setDeleting(true);
    setDeleteError(null);
    try {
      await onDelete();
      // On success the card unmounts — no local state to reset.
    } catch (err) {
      setDeleteError(errorMessage(err));
      setConfirmingDelete(false);
      deletedRef.current = false;
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    <div className={styles.cardBody}>
      <label className={styles.fieldLabel} htmlFor={`c-name-${customer.id}`}>
        Name
      </label>
      <input
        id={`c-name-${customer.id}`}
        className={styles.input}
        placeholder="Solo agency owners"
        value={name}
        onChange={(e) => setName(e.target.value)}
        disabled={deleting}
      />

      <Field
        id={`c-who-${customer.id}`}
        label="Who they are"
        hint="How you recognize one: their role, the shape of their company, where you find them."
        placeholder="Run a 1-5 person agency, sell services, no dedicated sales hire. Found through LinkedIn."
        value={whoTheyAre}
        onChange={setWhoTheyAre}
        disabled={deleting}
      />

      <Field
        id={`c-pain-${customer.id}`}
        label="What they care about"
        hint="What's broken for them today. This is what decides which snippets fit."
        placeholder="Outreach eats billable hours. They know they should follow up; they don't."
        value={pain}
        onChange={setPain}
        disabled={deleting}
      />

      <Field
        id={`c-goal-${customer.id}`}
        label="Goal"
        hint="Where a thread with them should land. The AI steers toward it — but it will never invent an ask that isn't in a snippet."
        placeholder="Get them on a 15-minute walkthrough call."
        value={goal}
        onChange={setGoal}
        disabled={deleting}
      />

      {(error || deleteError) && (
        <div className={styles.error} role="alert">
          {error ?? deleteError}
        </div>
      )}

      <div className={styles.cardActions}>
        <SavedIndicator visible={showSaved} />
        {saving && <span className={styles.savingHint}>Saving…</span>}
        <span className={styles.actionSpacer} aria-hidden="true" />
        {confirmingDelete ? (
          <>
            <span className={styles.confirmText}>
              Delete this profile? Its prospects stay in your pipeline.
            </span>
            <button
              type="button"
              className={styles.secondaryBtn}
              onClick={() => setConfirmingDelete(false)}
              disabled={deleting}
            >
              Cancel
            </button>
            <button
              type="button"
              className={styles.deleteBtn}
              onClick={() => void handleDelete()}
              disabled={deleting}
            >
              {deleting ? "Deleting…" : "Delete"}
            </button>
          </>
        ) : (
          <button
            type="button"
            className={styles.deleteTrigger}
            onClick={() => setConfirmingDelete(true)}
            disabled={deleting}
          >
            Delete
          </button>
        )}
      </div>
    </div>
  );
}

/** A labelled textarea with a line of guidance — the three steering fields all
 *  share this shape, and the guidance is what keeps them from collapsing into
 *  three ways of saying the same thing. */
function Field({
  id,
  label,
  hint,
  placeholder,
  value,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  hint: string;
  placeholder: string;
  value: string;
  onChange: (v: string) => void;
  disabled: boolean;
}) {
  return (
    <>
      <label className={styles.fieldLabel} htmlFor={id}>
        {label}
      </label>
      <p className={styles.hint}>{hint}</p>
      <textarea
        id={id}
        className={styles.textarea}
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
      />
    </>
  );
}

function Chevron({ expanded }: { expanded: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={styles.chevron}
      data-expanded={expanded || undefined}
    >
      <path
        d="m9 6 6 6-6 6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function GoalIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.goalIcon}>
      <circle cx="12" cy="12" r="8" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="12" cy="12" r="3" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.plusIcon}>
      <path
        d="M12 5v14M5 12h14"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}
