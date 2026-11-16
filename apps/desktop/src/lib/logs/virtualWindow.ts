/** Above this many rows the viewer renders only the rows in view. */
export const VIRTUALIZE_FROM = 500;

export interface VirtualWindow {
	/** First rendered row (inclusive). */
	start: number;
	/** Last rendered row (exclusive). */
	end: number;
	/** Space standing in for the rows above and below the rendered ones, in pixels. */
	padTop: number;
	padBottom: number;
}

export interface ViewportMetrics {
	count: number;
	rowHeight: number;
	scrollTop: number;
	viewportHeight: number;
	/** Extra rows rendered above and below the viewport, so fast scrolling shows no gaps. */
	overscan: number;
}

/** Rows to render for a fixed-height list scrolled to `scrollTop`. */
export function virtualWindow({ count, rowHeight, scrollTop, viewportHeight, overscan }: ViewportMetrics): VirtualWindow {
	if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
	const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
	const inView = Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1;
	const start = Math.min(count, Math.max(0, first - overscan));
	const end = Math.min(count, first + inView + overscan);
	return { start, end, padTop: start * rowHeight, padBottom: (count - end) * rowHeight };
}

/** Whether the scroller shows its last pixel, within `tolerance` (zoom rounds scroll offsets). */
export function isAtBottom(scrollTop: number, scrollHeight: number, clientHeight: number, tolerance: number): boolean {
	return scrollHeight - scrollTop - clientHeight <= tolerance;
}
