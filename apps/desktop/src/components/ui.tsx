import { ReactNode, useEffect, useRef, useState } from "react";
import {
  AlertIcon,
  CheckIcon,
  CopyIcon,
  GlobeIcon,
  TerminalIcon,
  XIcon,
} from "../icons";
import type { ServerStatus, ServerType } from "../types";

const STATUS_LABELS: Record<ServerStatus, string> = {
  stopped: "stopped",
  starting: "starting…",
  running: "running",
  stopping: "stopping…",
  error: "error",
};

export function StatusBadge({ status }: { status: ServerStatus }) {
  return (
    <span className={`status status-${status}`}>
      <span className="status-dot" />
      {STATUS_LABELS[status]}
    </span>
  );
}

const TYPE_META: Record<
  ServerType,
  { label: string; icon: (p: { size?: number }) => ReactNode }
> = {
  local: { label: "stdio", icon: TerminalIcon },
  remote: { label: "SSE", icon: GlobeIcon },
  "remote-streamable": { label: "HTTP", icon: GlobeIcon },
};

export function TypeBadge({ type }: { type: ServerType }) {
  const meta = TYPE_META[type];
  const Icon = meta.icon;
  return (
    <span className="badge">
      <Icon size={11} />
      {meta.label}
    </span>
  );
}

export function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
}) {
  return (
    <label className="switch">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.currentTarget.checked)}
      />
      <span className="switch-track" />
      {label && <span className="switch-label">{label}</span>}
    </label>
  );
}

export function CopyButton({
  value,
  className = "btn btn-sm icon-btn",
  title = "Copy",
}: {
  value: string;
  className?: string;
  title?: string;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can fail without focus; leave the button unchanged.
    }
  }

  return (
    <button type="button" className={className} onClick={copy} title={title}>
      {copied ? <CheckIcon size={14} /> : <CopyIcon size={14} />}
    </button>
  );
}

export function Banner({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss: () => void;
}) {
  return (
    <div className="banner" role="alert">
      <span className="banner-icon">
        <AlertIcon size={15} />
      </span>
      <div className="banner-body">{message}</div>
      <button
        type="button"
        className="banner-dismiss"
        onClick={onDismiss}
        aria-label="Dismiss"
      >
        <XIcon size={12} />
      </button>
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  note,
  action,
}: {
  icon: ReactNode;
  title: string;
  note: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">{icon}</span>
      <p className="empty-title">{title}</p>
      <p className="empty-note">{note}</p>
      {action}
    </div>
  );
}
