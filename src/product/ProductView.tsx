import { FormEvent, useState } from "react";
import {
  getProduct,
  polishProduct,
  updateProduct,
  type Product,
} from "../api/product";
import LoadError from "../components/LoadError";
import PolishButton from "../components/PolishButton";
import SavedIndicator from "../components/SavedIndicator";
import { useAutosave } from "../lib/useAutosave";
import { useLoad } from "../lib/useLoad";
import styles from "./ProductView.module.css";

/** Product tab: the one thing you sell, described once.
 *
 *  This is the invariant half of every message — the AI's single source of
 *  product truth. What varies per buyer lives in Customers, so nothing here
 *  should be written toward one audience. */
export default function ProductView() {
  const { data: product, error: loadError, retry } = useLoad(getProduct);

  if (loadError) {
    return <LoadError what="your product" detail={loadError} onRetry={retry} />;
  }

  return (
    <div className={styles.product}>
      <header className={styles.intro}>
        <h1 className={styles.title}>Product</h1>
        <p className={styles.subtitle}>
          The one thing you sell, written out once. Every draft and every comment
          is composed against this.
        </p>
      </header>

      {product ? (
        <ProductForm initial={product} />
      ) : (
        // Local SQLite resolves near-instantly; this reserves layout without a
        // flash of spinner on the common fast path.
        <div className={styles.loading} aria-busy="true" aria-hidden="true" />
      )}
    </div>
  );
}

function ProductForm({ initial }: { initial: Product }) {
  const [name, setName] = useState(initial.name);
  const [description, setDescription] = useState(initial.description);
  // A polish is in flight. Locks the field (and holds autosave) so the incoming
  // rewrite can't clobber text typed during the multi-second CLI call.
  const [polishing, setPolishing] = useState(false);

  const { saving, showSaved, dirty, error, setError, save } = useAutosave({
    values: { name, description },
    persist: (v) => updateProduct(v.name, v.description),
    hold: polishing,
  });

  // Manual save flushes immediately, skipping the debounce.
  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    save();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit}>
      <label className={styles.fieldLabel} htmlFor="product-name">
        Name
      </label>
      <input
        id="product-name"
        className={styles.input}
        placeholder="What your product is called"
        value={name}
        onChange={(e) => setName(e.target.value)}
        disabled={polishing}
      />

      <div className={styles.fieldHeader}>
        <label className={styles.fieldLabel} htmlFor="product-description">
          Description
        </label>
        <PolishButton
          text={description}
          polish={polishProduct}
          disabled={saving || polishing}
          onPolished={setDescription}
          onError={setError}
          onBusyChange={setPolishing}
        />
      </div>
      <p className={styles.hint}>
        What it is, who it's for, how it works, and why it beats the alternative.
        Write it broad — the angle for each kind of buyer belongs in{" "}
        <strong>Customers</strong>, not here.
      </p>
      <textarea
        id="product-description"
        className={styles.description}
        placeholder="The full story. Mechanics, numbers, integrations, proof, differentiators — everything the AI should know before it writes a word."
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        disabled={polishing}
      />

      {error && <div className={styles.error}>{error}</div>}

      <div className={styles.actions}>
        <SavedIndicator visible={showSaved} />
        <button
          type="submit"
          className={styles.primaryBtn}
          disabled={!dirty || saving || polishing}
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>
    </form>
  );
}
