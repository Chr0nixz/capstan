/**
 * Written locally before a reconnected session's output. The program that was
 * running when the connection dropped never got to undo its terminal modes,
 * so an interrupted vim, tmux or htop would keep sending mouse and focus
 * reports into the new prompt and keep right-click away from the context menu.
 */
export const RECONNECT_RESET = [
  // Cancel an escape sequence the drop cut off.
  "\x18",
  // Back to the normal screen.
  "\x1b[?1049l",
  // DECSTR: cursor visible, keypad, cursor keys, insert, origin, wrap,
  // scroll region, character sets and bracketed paste back to defaults.
  "\x1b[!p",
  // Mouse tracking off and its encodings back to legacy, which DECSTR keeps.
  "\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l",
  "\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l",
  // Focus reports off.
  "\x1b[?1004l",
  "\x1b[?2004l\x1b[0m",
].join("");
