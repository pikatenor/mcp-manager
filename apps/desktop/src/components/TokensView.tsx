import { FormEvent, useState } from "react";
import { AlertIcon, KeyIcon } from "../icons";
import type { TokenRecord } from "../types";
import { CopyButton, EmptyState } from "./ui";

function formatIssued(at: number): string {
  return new Date(at * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function TokensView({
  tokens,
  plaintext,
  onIssue,
  onRevoke,
  onDismissPlaintext,
}: {
  tokens: TokenRecord[];
  plaintext: string | null;
  onIssue: (clientName: string) => Promise<void>;
  onRevoke: (id: string) => Promise<void>;
  onDismissPlaintext: () => void;
}) {
  const [clientName, setClientName] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const activeCount = tokens.filter((t) => t.revoked_at === null).length;

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!clientName.trim()) return;
    setSubmitting(true);
    try {
      await onIssue(clientName.trim());
      setClientName("");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <>
      <header className="content-header">
        <div>
          <h1 className="content-title">Client tokens</h1>
          <p className="content-subtitle">
            {activeCount === 0
              ? "Bearer tokens for clients connecting to the endpoint"
              : `${activeCount} active of ${tokens.length} issued`}
          </p>
        </div>
      </header>
      <div className="content-body">
        <form className="field issue-field" onSubmit={handleSubmit}>
          <label className="field-label" htmlFor="client-name">
            New token
          </label>
          <div className="issue-controls">
            <input
              id="client-name"
              value={clientName}
              onChange={(e) => setClientName(e.currentTarget.value)}
              placeholder="client name, e.g. cursor"
            />
            <button
              type="submit"
              className="btn btn-primary"
              disabled={submitting || !clientName.trim()}
            >
              Issue token
            </button>
          </div>
        </form>

        {plaintext && (
          <div className="secret-callout">
            <span className="secret-callout-icon">
              <AlertIcon size={16} />
            </span>
            <div className="secret-callout-body">
              <p className="secret-callout-title">
                Copy this token now — it will not be shown again
              </p>
              <p className="secret-callout-note">
                Only a hash is stored; the plaintext exists just this once.
              </p>
              <div className="secret-value">
                <code>{plaintext}</code>
                <CopyButton value={plaintext} title="Copy token" />
                <button
                  type="button"
                  className="btn btn-sm btn-ghost"
                  onClick={onDismissPlaintext}
                >
                  Dismiss
                </button>
              </div>
            </div>
          </div>
        )}

        {tokens.length === 0 ? (
          <EmptyState
            icon={<KeyIcon size={28} />}
            title="No tokens issued"
            note="Issue a bearer token for each client that connects to this endpoint."
          />
        ) : (
          <div className="card token-list">
            {tokens.map((token) => (
              <div className="token-row" key={token.id}>
                <div className="token-identity">
                  <span className="token-icon">
                    <KeyIcon size={13} />
                  </span>
                  <div>
                    <div className="token-name">{token.client_name}</div>
                    <div className="token-date">
                      Issued {formatIssued(token.issued_at)}
                    </div>
                  </div>
                </div>
                <div className="token-meta">
                  {token.revoked_at ? (
                    <span className="badge">revoked</span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      onClick={() => onRevoke(token.id)}
                    >
                      Revoke
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </>
  );
}
