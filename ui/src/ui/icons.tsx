import type { ReactElement, ReactNode } from "react";

// Authored line icons on a 16px grid. One stroke weight, round caps, and
// currentColor keep every control in the same visual family.
type IconProps = { size?: number; className?: string };

function icon(paths: ReactNode) {
  return function Icon({ size = 16, className }: IconProps): ReactElement {
    return <svg className={"icon" + (className ? ` ${className}` : "")} width={size} height={size} viewBox="0 0 16 16"
      fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {paths}
    </svg>;
  };
}

export const CloseIcon = icon(<path d="m4 4 8 8M12 4l-8 8" />);
export const PlusIcon = icon(<path d="M8 3v10M3 8h10" />);
export const ChevronDownIcon = icon(<path d="m4.5 6.5 3.5 3.5 3.5-3.5" />);
export const ChevronRightIcon = icon(<path d="m6.5 4.5 3.5 3.5-3.5 3.5" />);
export const SearchIcon = icon(<><circle cx="7" cy="7" r="4.25" /><path d="m10.25 10.25 3.25 3.25" /></>);
export const LockIcon = icon(<><rect x="3.25" y="7" width="9.5" height="6.5" rx="1.5" /><path d="M5.25 7V5.25a2.75 2.75 0 0 1 5.5 0V7" /></>);
export const UnlockIcon = icon(<><rect x="3.25" y="7" width="9.5" height="6.5" rx="1.5" /><path d="M5.25 7V5.25a2.75 2.75 0 0 1 5.3-1" /></>);
export const SettingsIcon = icon(<><path d="M6.6 2.2h2.8l.4 1.6 1.2.7 1.6-.5 1.4 2.4-1.2 1.1v1.4l1.2 1.1-1.4 2.4-1.6-.5-1.2.7-.4 1.6H6.6l-.4-1.6-1.2-.7-1.6.5L2 10.5l1.2-1.1V8L2 6.9l1.4-2.4 1.6.5 1.2-.7Z" /><circle cx="8" cy="8" r="1.9" /></>);
export const ServerIcon = icon(<><rect x="2.5" y="2.75" width="11" height="4.5" rx="1.25" /><rect x="2.5" y="8.75" width="11" height="4.5" rx="1.25" /><path d="M5 5h.01M5 11h.01" strokeWidth="1.8" /></>);
export const TerminalIcon = icon(<><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.75" /><path d="m4.5 6.25 2 1.75-2 1.75M8 10h3.25" /></>);
export const FolderIcon = icon(<path d="M2 4.25c0-.7.55-1.25 1.25-1.25h2.6l1.4 1.5h5.5c.7 0 1.25.55 1.25 1.25v5.75c0 .7-.55 1.25-1.25 1.25h-9.5C2.55 12.75 2 12.2 2 11.5Z" />);
export const FileIcon = icon(<><path d="M4.25 1.75h4.5l3 3v8.25c0 .7-.55 1.25-1.25 1.25h-6.25C3.55 14.25 3 13.7 3 13V3c0-.7.55-1.25 1.25-1.25Z" /><path d="M8.75 1.75V4.75h3" /></>);
export const LinkIcon = icon(<><path d="M6.75 9.25a2.5 2.5 0 0 0 3.55 0l2.2-2.2a2.5 2.5 0 0 0-3.55-3.55l-.7.7" /><path d="M9.25 6.75a2.5 2.5 0 0 0-3.55 0l-2.2 2.2a2.5 2.5 0 0 0 3.55 3.55l.7-.7" /></>);
export const ArrowUpIcon = icon(<path d="M8 13V3.5M4 7.25 8 3.25l4 4" />);
export const ArrowDownIcon = icon(<path d="M8 3v9.5M4 8.75l4 4 4-4" />);
export const RefreshIcon = icon(<><path d="M13 8a5 5 0 1 1-1.5-3.55" /><path d="M13 2.75V5.5h-2.75" /></>);
export const EyeIcon = icon(<><path d="M1.75 8S4 3.75 8 3.75 14.25 8 14.25 8 12 12.25 8 12.25 1.75 8 1.75 8Z" /><circle cx="8" cy="8" r="1.9" /></>);
export const CopyIcon = icon(<><rect x="5.25" y="5.25" width="8" height="8" rx="1.5" /><path d="M10.75 5.25V4c0-.7-.55-1.25-1.25-1.25H4c-.7 0-1.25.55-1.25 1.25v5.5c0 .7.55 1.25 1.25 1.25h1.25" /></>);
export const CheckIcon = icon(<path d="m3.25 8.5 3 3 6.5-7" />);
export const UploadIcon = icon(<><path d="M8 10.5V2.75M4.75 6 8 2.75 11.25 6" /><path d="M2.75 10.5v1.75c0 .55.45 1 1 1h8.5c.55 0 1-.45 1-1V10.5" /></>);
export const DownloadIcon = icon(<><path d="M8 2.75v7.75M4.75 7.25 8 10.5l3.25-3.25" /><path d="M2.75 10.5v1.75c0 .55.45 1 1 1h8.5c.55 0 1-.45 1-1V10.5" /></>);
export const FolderUploadIcon = icon(<><path d="M2 4.25c0-.7.55-1.25 1.25-1.25h2.6l1.4 1.5h5.5c.7 0 1.25.55 1.25 1.25v5.75c0 .7-.55 1.25-1.25 1.25h-9.5C2.55 12.75 2 12.2 2 11.5Z" /><path d="M8 10.75v-4M6.25 8.25 8 6.5l1.75 1.75" /></>);
export const FolderDownloadIcon = icon(<><path d="M2 4.25c0-.7.55-1.25 1.25-1.25h2.6l1.4 1.5h5.5c.7 0 1.25.55 1.25 1.25v5.75c0 .7-.55 1.25-1.25 1.25h-9.5C2.55 12.75 2 12.2 2 11.5Z" /><path d="M8 6.5v4M6.25 9 8 10.75 9.75 9" /></>);
export const FilePlusIcon = icon(<><path d="M8.75 1.75h-4.5C3.55 1.75 3 2.3 3 3v10c0 .7.55 1.25 1.25 1.25h6.25c.7 0 1.25-.55 1.25-1.25V4.75Z" /><path d="M8.75 1.75V4.75h3M7.4 7.5v4.25M5.25 9.6H9.5" /></>);
export const FolderPlusIcon = icon(<><path d="M2 4.25c0-.7.55-1.25 1.25-1.25h2.6l1.4 1.5h5.5c.7 0 1.25.55 1.25 1.25v5.75c0 .7-.55 1.25-1.25 1.25h-9.5C2.55 12.75 2 12.2 2 11.5Z" /><path d="M8 6.5v4M6 8.5h4" /></>);
export const PencilIcon = icon(<><path d="m10.5 2.75 2.75 2.75-7.5 7.5H3v-2.75Z" /><path d="m9 4.25 2.75 2.75" /></>);
export const TrashIcon = icon(<><path d="M2.75 4.25h10.5M6.25 4.25V2.75h3.5v1.5" /><path d="M4 4.25 4.75 13c.05.6.55 1 1.1 1h4.3c.55 0 1.05-.4 1.1-1L12 4.25M6.75 7v4.25M9.25 7v4.25" /></>);
export const KeyIcon = icon(<><circle cx="5.25" cy="10.75" r="2.75" /><path d="m7.25 8.75 5.5-5.5M10.75 5.25l1.75 1.75M9.25 6.75l1.25 1.25" /></>);
export const ExternalEditIcon = icon(<><path d="M9 2.75h4.25V7M13.25 2.75 7.5 8.5" /><path d="M11.75 9.5v2.75c0 .7-.55 1.25-1.25 1.25h-6.5c-.7 0-1.25-.55-1.25-1.25v-6.5c0-.7.55-1.25 1.25-1.25H6.5" /></>);
export const ShellIcon = icon(<><path d="M2.75 4.25 6.5 8l-3.75 3.75" /><path d="M8.5 12h4.75" /></>);
export const ExportIcon = icon(<><rect x="2.25" y="6.25" width="8" height="7.5" rx="1.25" /><path d="M8.25 2.25h5.5v5.5M13.75 2.25 8.5 7.5" /></>);
export const StarIcon = icon(<path d="m8 2 1.8 3.7 4 .55-2.9 2.8.7 4L8 11.15 4.4 13.05l.7-4-2.9-2.8 4-.55Z" />);
export const AlertIcon = icon(<><path d="M7.13 2.6a1 1 0 0 1 1.74 0l5.5 9.85a1 1 0 0 1-.87 1.5H2.5a1 1 0 0 1-.87-1.5Z" /><path d="M8 6.25v3M8 11.5h.01" /></>);
export const ShieldIcon = icon(<><path d="M8 1.75 13 3.5v4c0 3.1-2.1 5.6-5 6.75C5.1 13.1 3 10.6 3 7.5v-4Z" /><path d="m5.75 8 1.6 1.6 2.9-3.1" /></>);
export const FollowIcon = icon(<><circle cx="8" cy="8" r="2" /><path d="M8 1.75v2.5M8 11.75v2.5M1.75 8h2.5M11.75 8h2.5" /></>);
export const CollapseIcon = icon(<><rect x="2" y="2.75" width="12" height="10.5" rx="1.75" /><path d="M10 2.75v10.5M5.25 6.5 6.75 8l-1.5 1.5" /></>);
export const PauseIcon = icon(<path d="M5.75 3.5v9M10.25 3.5v9" />);
export const PlayIcon = icon(<path d="M5 3.25v9.5L12.5 8Z" />);
export const RetryIcon = icon(<><path d="M3 8a5 5 0 1 0 1.5-3.55" /><path d="M3 2.75V5.5h2.75" /></>);
export const MinimizeGlyph = icon(<path d="M3 8.5h10" strokeWidth="1" />);
export const MaximizeGlyph = icon(<rect x="3.5" y="3.5" width="9" height="9" rx="1" strokeWidth="1" />);
export const CloseGlyph = icon(<path d="m3.5 3.5 9 9M12.5 3.5l-9 9" strokeWidth="1" />);
export const MoreIcon = icon(<path d="M3.5 8h.01M8 8h.01M12.5 8h.01" strokeWidth="2.2" />);
export const ArrowRightIcon = icon(<path d="M3 8h9.5M8.75 4.25 12.5 8l-3.75 3.75" />);
export const SidebarIcon = icon(<><rect x="2" y="2.75" width="12" height="10.5" rx="1.75" /><path d="M6.25 2.75v10.5" /></>);
// Pane splits draw two panes with the new one filled in, so they never read as
// the file panel's dock keys.
export const SplitRightIcon = icon(<><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.75" /><path d="M8 2.75v10.5" /><path d="M10 6.25v3.5M8.25 8h3.5" /></>);
export const SplitDownIcon = icon(<><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.75" /><path d="M1.75 8h12.5" /><path d="M8 9.25v2.75M6.6 10.6h2.8" /></>);
// One source fanning out to three panes: input sent to every pane at once.
export const BroadcastIcon = icon(<><circle cx="3.5" cy="8" r="1.5" /><path d="M5 8h2.5M7.5 8l4-4.25M7.5 8h4M7.5 8l4 4.25" /><path d="M12.75 3.75h.01M12.75 8h.01M12.75 12.25h.01" strokeWidth="2.4" /></>);
// Snippets are saved commands: a bookmark holding a prompt, so the key never
// reads as the terminal glyph beside it.
export const SnippetIcon = icon(<><path d="M4 2.25h8c.4 0 .75.35.75.75v10.75L8 10.75l-4.75 3V3c0-.4.35-.75.75-.75Z" /><path d="m6 5.25 1.6 1.5L6 8.25M8.75 8.25h1.5" /></>);
// A forwarded port: a connection passing through an arch.
export const TunnelIcon = icon(<><path d="M2.25 13.25V8.5a5.75 5.75 0 0 1 11.5 0v4.75" /><path d="M5 10.5h6M9.25 8.75 11 10.5l-1.75 1.75" /></>);
// The command palette: a search field over a short list of actions.
export const CommandIcon = icon(<><rect x="1.75" y="2.25" width="12.5" height="4" rx="1.25" /><path d="M4 9.25h8M4 12.25h5.5" /></>);
// Terminal themes: a half-lit disc.
export const ThemeIcon = icon(<><circle cx="8" cy="8" r="5.75" /><path d="M8 2.25v11.5a5.75 5.75 0 0 0 0-11.5Z" fill="currentColor" stroke="none" /></>);
export const DockRightIcon = icon(<><rect x="2" y="2.75" width="12" height="10.5" rx="1.75" /><path d="M9.5 2.75v10.5" /></>);
export const DockBottomIcon = icon(<><rect x="2" y="2.75" width="12" height="10.5" rx="1.75" /><path d="M2 9.25h12" /></>);
