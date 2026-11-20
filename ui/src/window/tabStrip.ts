import { useLayoutEffect, type RefObject } from "react";

/** Room left beside a revealed tab so it never sits under the edge fade. */
export const TAB_EDGE_FADE = 24;

/**
 * The scroll offset that brings the item at [start, end) into a viewport of
 * `width` currently scrolled to `scroll`. An item wider than the viewport
 * shows its leading edge. Offsets already in view are returned unchanged.
 */
export function revealScroll(scroll: number, width: number, start: number, end: number, margin = 0): number {
  if (start - margin < scroll) return Math.max(0, start - margin);
  if (end + margin > scroll + width) return Math.max(0, Math.min(start - margin, end + margin - width));
  return scroll;
}

/** Which edges of a horizontal scroller have content hidden past them. */
export function overflowEdges(scroll: number, scrollWidth: number, width: number): { start: boolean; end: boolean } {
  return { start: scroll > 1, end: scroll + width < scrollWidth - 1 };
}

/**
 * Keeps the active terminal tab inside the visible part of the tab list,
 * marks the edges that have tabs scrolled past them, and lets a vertical
 * wheel scroll the strip sideways. The list's scrollbar stays hidden, so
 * the edge marks are the only overflow cue.
 */
export function useTabStrip(list: RefObject<HTMLElement | null>, activeId: string | null, count: number): void {
  useLayoutEffect(() => {
    const element = list.current;
    if (!element) return;
    const sync = (): void => {
      const edges = overflowEdges(element.scrollLeft, element.scrollWidth, element.clientWidth);
      element.toggleAttribute("data-overflow-start", edges.start);
      element.toggleAttribute("data-overflow-end", edges.end);
    };
    const reveal = (): void => {
      const tab = element.querySelector<HTMLElement>(".terminal-tab.active");
      if (tab) {
        const box = element.getBoundingClientRect(), item = tab.getBoundingClientRect();
        const start = item.left - box.left + element.scrollLeft;
        const next = revealScroll(element.scrollLeft, element.clientWidth, start, start + item.width, TAB_EDGE_FADE);
        if (next !== element.scrollLeft) element.scrollLeft = next;
      }
      sync();
    };
    const wheel = (event: WheelEvent): void => {
      if (event.ctrlKey || Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      if (element.scrollWidth <= element.clientWidth) return;
      element.scrollLeft += event.deltaY;
    };
    reveal();
    element.addEventListener("scroll", sync, { passive: true });
    element.addEventListener("wheel", wheel, { passive: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(reveal);
    observer?.observe(element);
    return () => {
      element.removeEventListener("scroll", sync);
      element.removeEventListener("wheel", wheel);
      observer?.disconnect();
    };
  }, [list, activeId, count]);
}
