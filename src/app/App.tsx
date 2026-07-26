import { useState } from "react";
import Tabs, { type TabItem } from "../components/Tabs";
import CommentsView from "../comments/CommentsView";
import CustomersView from "../customers/CustomersView";
import ProductView from "../product/ProductView";
import ProfileView from "../profile/ProfileView";
import ProspectsView from "../prospects/ProspectsView";
import SnippetsView from "../snippets/SnippetsView";
import styles from "./app.module.css";

type TabId =
  | "product"
  | "customers"
  | "prospects"
  | "snippets"
  | "comments"
  | "profile";

/**
 * The four tabs of the sale, left to right: what you sell, who you sell it to,
 * who you're working right now, and what you actually say. Comments and Profile
 * are pushed to the far right — they're about you and your presence, not a deal.
 */
const LEFT_TABS: TabItem<TabId>[] = [
  { id: "product", label: "Product" },
  { id: "customers", label: "Customers" },
  { id: "prospects", label: "Prospects" },
  { id: "snippets", label: "Snippets" },
];

const RIGHT_TABS: TabItem<TabId>[] = [
  { id: "comments", label: "Comments" },
  { id: "profile", label: "Profile" },
];

/**
 * The app surface: navbar and the content of the active tab.
 *
 * Nothing here is scoped any more. There is one product, one pipeline, and one
 * snippet library, so the old active-pitch context (a switcher in the navbar,
 * threaded into every view) has no reason to exist — the only state left is
 * which tab is showing, and each view loads its own data.
 */
export default function App() {
  // Prospects is the daily driver — the board you come back to.
  const [activeTab, setActiveTab] = useState<TabId>("prospects");

  return (
    <div className={styles.app}>
      <header className={styles.navbar}>
        <div className={styles.navInner}>
          <Tabs items={LEFT_TABS} active={activeTab} onChange={setActiveTab} />
          <span className={styles.navSpacer} aria-hidden="true" />
          <Tabs items={RIGHT_TABS} active={activeTab} onChange={setActiveTab} />
        </div>
      </header>

      <main className={styles.content}>
        {activeTab === "product" && <ProductView />}
        {activeTab === "customers" && <CustomersView />}
        {activeTab === "prospects" && <ProspectsView />}
        {activeTab === "snippets" && <SnippetsView />}
        {activeTab === "comments" && <CommentsView />}
        {activeTab === "profile" && <ProfileView />}
      </main>
    </div>
  );
}
