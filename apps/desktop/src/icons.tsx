import { ReactNode } from "react";

type IconProps = {
  size?: number;
  className?: string;
};

function Icon({
  size = 16,
  className,
  children,
}: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** Hub-and-nodes mark used for the brand tile and favicon. */
export function HubIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="2" fill="currentColor" stroke="none" />
      <circle cx="2.8" cy="3.4" r="1.3" />
      <circle cx="13.2" cy="3.4" r="1.3" />
      <circle cx="8" cy="13.4" r="1.3" />
      <path d="M7 6.7 3.6 4.5M9 6.7l3.4-2.2M8 10v1.9" />
    </Icon>
  );
}

export function ServerIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="1.8" y="2.2" width="12.4" height="5" rx="1.4" />
      <rect x="1.8" y="8.8" width="12.4" height="5" rx="1.4" />
      <path d="M4.3 4.7h.01M4.3 11.3h.01" strokeWidth="2.2" />
    </Icon>
  );
}

export function KeyIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="5.5" cy="10.5" r="3.7" />
      <path d="M8.2 7.8 13.5 2.5M11 5l2 2M13 3l2 2" />
    </Icon>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 3v10M3 8h10" />
    </Icon>
  );
}

export function CopyIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M10.5 5.5V4A1.5 1.5 0 0 0 9 2.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5" />
    </Icon>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M13 5.5 6.5 12 3 8.5" />
    </Icon>
  );
}

export function TrashIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2.5 4.5h11M6.5 4.5V3.2A1.2 1.2 0 0 1 7.7 2h.6a1.2 1.2 0 0 1 1.2 1.2v1.3M12.5 4.5l-.7 8.4a1.5 1.5 0 0 1-1.5 1.4H5.7a1.5 1.5 0 0 1-1.5-1.4L3.5 4.5" />
    </Icon>
  );
}

export function PlayIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5.5 3.5v9l7.5-4.5-7.5-4.5z" />
    </Icon>
  );
}

export function StopIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" />
    </Icon>
  );
}

export function RefreshIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5V5h-2.5" />
    </Icon>
  );
}

export function AlertIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6.9 3 1.7 12.2A1.3 1.3 0 0 0 2.8 14.1h10.4a1.3 1.3 0 0 0 1.1-1.9L9.1 3a1.3 1.3 0 0 0-2.2 0z" />
      <path d="M8 6.3v3M8 11.5h.01" strokeWidth="2" />
    </Icon>
  );
}

export function LockIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="7" width="10" height="7" rx="1.5" />
      <path d="M5.3 7V5a2.7 2.7 0 0 1 5.4 0v2" />
    </Icon>
  );
}

export function XIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m4 4 8 8M12 4l-8 8" />
    </Icon>
  );
}

export function TerminalIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m3 5 3.5 3L3 11M8 12h5" />
    </Icon>
  );
}

export function GlobeIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="6.2" />
      <path d="M1.8 8h12.4M8 1.8c1.7 1.7 2.6 3.9 2.6 6.2S9.7 12.5 8 14.2C6.3 12.5 5.4 10.3 5.4 8S6.3 3.5 8 1.8z" />
    </Icon>
  );
}

export function InboxIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2.5 9.5 4.7 3.4a1.2 1.2 0 0 1 1.1-.9h4.4a1.2 1.2 0 0 1 1.1.9l2.2 6.1v2.7a1.3 1.3 0 0 1-1.3 1.3H3.8a1.3 1.3 0 0 1-1.3-1.3V9.5z" />
      <path d="M2.5 9.5h3l.8 1.6h3.4l.8-1.6h3" />
    </Icon>
  );
}
