export type TooltipPlacement = 'top' | 'bottom' | 'auto';

export type TooltipParams = {
  text: string;
  /**
   * Delay before showing the tooltip.
   * Default: 1000ms (i.e. "hover N seconds" where N=1 by default).
   */
  delayMs?: number;
  placement?: TooltipPlacement;
  offsetPx?: number;
  maxWidthPx?: number;
  disabled?: boolean;
  /**
   * If true, keep the native `title` behavior (not recommended).
   * Default: false (we suppress native tooltip to avoid instant popups).
   */
  preserveNativeTitle?: boolean;
};

const DEFAULT_DELAY_MS = 1000;
const DEFAULT_OFFSET_PX = 10;
const DEFAULT_MAX_WIDTH_PX = 360;
const VIEWPORT_MARGIN_PX = 8;

let stylesInjected = false;

function ensureTooltipStyles() {
  if (stylesInjected) return;
  if (typeof document === 'undefined') return;
  if (document.getElementById('dh-tooltip-styles')) {
    stylesInjected = true;
    return;
  }
  const style = document.createElement('style');
  style.id = 'dh-tooltip-styles';
  style.textContent = `
.dh-tooltip {
  position: fixed;
  z-index: 2147483647;
  background: rgba(17, 24, 39, 0.95);
  color: #f9fafb;
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-radius: 10px;
  padding: 6px 10px;
  font-size: 12px;
  line-height: 1.4;
  max-width: 360px;
  box-shadow: 0 10px 26px rgba(0,0,0,0.22), 0 2px 10px rgba(0,0,0,0.14);
  pointer-events: none;
  user-select: none;
  opacity: 0;
  transform: translateY(2px);
  transition: opacity 120ms ease, transform 120ms ease;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
}
.dh-tooltip.dh-tooltip--open {
  opacity: 1;
  transform: translateY(0);
}
  `.trim();
  document.head.appendChild(style);
  stylesInjected = true;
}

function clamp(n: number, min: number, max: number) {
  return Math.min(max, Math.max(min, n));
}

function normalizeParams(p: TooltipParams): Required<TooltipParams> {
  return {
    text: String(p?.text ?? ''),
    delayMs: Number.isFinite(p?.delayMs) ? (p.delayMs as number) : DEFAULT_DELAY_MS,
    placement: (p?.placement ?? 'auto') as TooltipPlacement,
    offsetPx: Number.isFinite(p?.offsetPx) ? (p.offsetPx as number) : DEFAULT_OFFSET_PX,
    maxWidthPx: Number.isFinite(p?.maxWidthPx) ? (p.maxWidthPx as number) : DEFAULT_MAX_WIDTH_PX,
    disabled: Boolean(p?.disabled ?? false),
    preserveNativeTitle: Boolean(p?.preserveNativeTitle ?? false)
  };
}

function nextId() {
  return `dh-tip-${Math.random().toString(16).slice(2)}-${Date.now().toString(16)}`;
}

export function tooltip(node: HTMLElement, params: TooltipParams) {
  ensureTooltipStyles();

  let p = normalizeParams(params);
  let timer: number | null = null;
  let tip: HTMLDivElement | null = null;
  const tipId = nextId();

  let originalTitle: string | null = null;
  let titleSuppressed = false;

  const winListeners: Array<[keyof WindowEventMap, (e: any) => void, AddEventListenerOptions?]> = [];

  function clearTimer() {
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
  }

  function suppressNativeTitle() {
    if (p.preserveNativeTitle) return;
    if (titleSuppressed) return;
    if (!node.hasAttribute('title')) return;
    originalTitle = node.getAttribute('title');
    node.removeAttribute('title');
    titleSuppressed = true;
  }

  function restoreNativeTitle() {
    if (!titleSuppressed) return;
    if (p.preserveNativeTitle) return;
    if (originalTitle !== null) node.setAttribute('title', originalTitle);
    originalTitle = null;
    titleSuppressed = false;
  }

  function removeWindowListeners() {
    for (const [name, fn, opt] of winListeners) window.removeEventListener(name, fn, opt);
    winListeners.length = 0;
  }

  function positionTooltip() {
    if (!tip) return;
    // Apply per-instance max width.
    tip.style.maxWidth = `${p.maxWidthPx}px`;

    const rect = node.getBoundingClientRect();

    // If node is effectively offscreen, hide.
    if (rect.width === 0 && rect.height === 0) return;

    // Measure tooltip after setting maxWidth.
    const tr = tip.getBoundingClientRect();

    const vw = window.innerWidth || document.documentElement.clientWidth || 0;
    const vh = window.innerHeight || document.documentElement.clientHeight || 0;

    const centerX = rect.left + rect.width / 2;
    let x = centerX - tr.width / 2;
    x = clamp(x, VIEWPORT_MARGIN_PX, Math.max(VIEWPORT_MARGIN_PX, vw - tr.width - VIEWPORT_MARGIN_PX));

    const yTop = rect.top - tr.height - p.offsetPx;
    const yBottom = rect.bottom + p.offsetPx;

    let y = yTop;
    if (p.placement === 'bottom') y = yBottom;
    else if (p.placement === 'top') y = yTop;
    else {
      // auto: prefer top, but fall back to bottom if not enough space.
      const topOk = yTop >= VIEWPORT_MARGIN_PX;
      const bottomOk = yBottom + tr.height <= vh - VIEWPORT_MARGIN_PX;
      if (!topOk && bottomOk) y = yBottom;
      else y = yTop;
    }

    // Clamp y to viewport (best-effort).
    y = clamp(y, VIEWPORT_MARGIN_PX, Math.max(VIEWPORT_MARGIN_PX, vh - tr.height - VIEWPORT_MARGIN_PX));

    tip.style.left = `${Math.round(x)}px`;
    tip.style.top = `${Math.round(y)}px`;
  }

  function showNow() {
    if (tip) return;
    if (p.disabled) return;
    const text = (p.text ?? '').trim();
    if (!text) return;

    suppressNativeTitle();

    tip = document.createElement('div');
    tip.className = 'dh-tooltip';
    tip.id = tipId;
    tip.textContent = text;
    document.body.appendChild(tip);

    node.setAttribute('aria-describedby', tipId);

    positionTooltip();

    const onReposition = () => positionTooltip();
    // Scroll can happen on any container; `capture` helps for nested scroll areas.
    window.addEventListener('scroll', onReposition, { passive: true, capture: true });
    window.addEventListener('resize', onReposition, { passive: true });
    winListeners.push(['scroll', onReposition, { passive: true, capture: true }], ['resize', onReposition, { passive: true }]);

    // Transition in.
    requestAnimationFrame(() => {
      tip?.classList.add('dh-tooltip--open');
      positionTooltip();
    });
  }

  function hide() {
    clearTimer();
    if (tip) {
      tip.remove();
      tip = null;
      removeWindowListeners();
      if (node.getAttribute('aria-describedby') === tipId) node.removeAttribute('aria-describedby');
    }
    restoreNativeTitle();
  }

  function scheduleShow() {
    if (p.disabled) return;
    const text = (p.text ?? '').trim();
    if (!text) return;
    suppressNativeTitle();
    clearTimer();
    timer = window.setTimeout(() => {
      timer = null;
      showNow();
    }, p.delayMs);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape') hide();
  }

  node.addEventListener('mouseenter', scheduleShow);
  node.addEventListener('mouseleave', hide);
  node.addEventListener('focusin', scheduleShow);
  node.addEventListener('focusout', hide);
  node.addEventListener('pointerdown', hide);
  node.addEventListener('keydown', onKeyDown);

  return {
    update(next: TooltipParams) {
      p = normalizeParams(next);
      if (tip) {
        tip.textContent = (p.text ?? '').trim();
        positionTooltip();
      }
      // If disabled while visible, hide immediately.
      if (p.disabled) hide();
    },
    destroy() {
      hide();
      node.removeEventListener('mouseenter', scheduleShow);
      node.removeEventListener('mouseleave', hide);
      node.removeEventListener('focusin', scheduleShow);
      node.removeEventListener('focusout', hide);
      node.removeEventListener('pointerdown', hide);
      node.removeEventListener('keydown', onKeyDown);
      removeWindowListeners();
      // Ensure native title restored if we suppressed it.
      restoreNativeTitle();
    }
  };
}

