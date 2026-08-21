import { HubIcon, KeyIcon, ServerIcon } from "../icons";
import { CopyButton } from "./ui";

export type View = "servers" | "tokens";

export function Sidebar({
  view,
  onViewChange,
  serverCount,
  tokenCount,
  endpoint,
}: {
  view: View;
  onViewChange: (view: View) => void;
  serverCount: number;
  tokenCount: number;
  endpoint: string;
}) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark">
          <HubIcon size={17} />
        </span>
        <span className="brand-name">MCP Manager</span>
      </div>

      <nav className="nav">
        <button
          type="button"
          className={`nav-item${view === "servers" ? " active" : ""}`}
          onClick={() => onViewChange("servers")}
        >
          <span className="nav-item-icon">
            <ServerIcon size={15} />
          </span>
          Servers
          <span className="nav-count">{serverCount}</span>
        </button>
        <button
          type="button"
          className={`nav-item${view === "tokens" ? " active" : ""}`}
          onClick={() => onViewChange("tokens")}
        >
          <span className="nav-item-icon">
            <KeyIcon size={15} />
          </span>
          Tokens
          <span className="nav-count">{tokenCount}</span>
        </button>
      </nav>

      <div className="sidebar-footer">
        <div className="endpoint-box">
          <span className="endpoint-label">Aggregator endpoint</span>
          <div className="endpoint-url">
            <code title={endpoint}>{endpoint}</code>
            <CopyButton value={endpoint} title="Copy endpoint URL" />
          </div>
        </div>
        <p className="sidebar-note">
          Closing the window hides the app to the menu bar.
        </p>
      </div>
    </aside>
  );
}
