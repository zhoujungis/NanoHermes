/**
 * 图标集合：统一 1.65 描边、currentColor、24 网格。
 * 内联 path 而不是引入图标库，避免多一份依赖和首屏抖动。
 */
export const ICONS = {
  panelLeft: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9.5 4v16"/>',
  panelRight: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M14.5 4v16"/>',
  gear:
    '<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M4.6 7.4l2 1.2M17.4 15.4l2 1.2M4.6 16.6l2-1.2M17.4 8.6l2-1.2"/>',
  minus: '<path d="M5 12h14"/>',
  square: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  plus: '<path d="M12 5.5v13M5.5 12h13"/>',
  arrowUp: '<path d="M12 19V5M6 11l6-6 6 6"/>',
  arrowDown: '<path d="M12 5v14M6 13l6 6 6-6"/>',
  chev: '<path d="M9.5 6l6 6-6 6"/>',
  pencil: '<path d="M4 20l3.6-.8L18.4 8.4a2 2 0 0 0-2.8-2.8L4.8 16.4 4 20z"/>',
  edit: '<path d="M4 20l3.6-.8L18.4 8.4a2 2 0 0 0-2.8-2.8L4.8 16.4 4 20z"/>',
  refresh: '<path d="M20 12a8 8 0 1 1-2.3-5.6"/><path d="M20 4.5V10h-5.5"/>',
  copy:
    '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M15 5.5A2.5 2.5 0 0 0 12.5 3H6a3 3 0 0 0-3 3v6.5A2.5 2.5 0 0 0 5.5 15"/>',
  dot3: '<circle cx="12" cy="6" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="12" cy="18" r="1.3"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5z"/><path d="M14 3v5h5"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="M20 20l-3.6-3.6"/>',
  spark: '<path d="M12 3l1.7 5.2L19 10l-5.3 1.8L12 17l-1.7-5.2L5 10l5.3-1.8L12 3z"/>',
  terminal: '<path d="M5 8l3.5 3.5L5 15"/><path d="M12 15h6"/>',
  trash: '<path d="M4 7h16M9.5 7V5h5v2M6 7l1 13h10l1-13"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  alert: '<path d="M12 9v4M12 17h.01"/><circle cx="12" cy="12" r="9"/>',
  box: '<path d="M3 8l9-4.5L21 8v8l-9 4.5L3 16V8z"/><path d="M3 8l9 4.5 9-4.5"/>',
  shield:
    '<path d="M12 3l7 3v6c0 4.2-2.9 7.4-7 9-4.1-1.6-7-4.8-7-9V6l7-3z"/>',
  diff: '<path d="M9 4v16M15 4v16"/><path d="M4 8h5M15 16h5"/>',
  cloud: '<path d="M17.5 19H9a5 5 0 1 1 .9-9.9A6 6 0 0 1 21 11.6 3.75 3.75 0 0 1 17.5 19z"/>',
  laptop: '<rect x="3" y="5" width="18" height="11" rx="2"/><path d="M2 19h20"/>',
  folder:
    '<path d="M3 7.5A2 2 0 0 1 5 5.5h3.6l1.8 2H19a2 2 0 0 1 2 2v7A2 2 0 0 1 19 16.5H5a2 2 0 0 1-2-2v-7z"/>',
  download: '<path d="M12 4v11"/><path d="M7.5 11l4.5 4.5L16.5 11"/><path d="M5 19.5h14"/>',
  image:
    '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><circle cx="9" cy="10" r="1.6"/><path d="M4.5 17.2l4.6-4.6 3.3 3.3 2.6-2.4 4.5 4.3"/>',
  theme:
    '<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 0 1 0 17z" fill="currentColor" stroke="none"/>',
  user: '<circle cx="12" cy="8" r="3.6"/><path d="M5.5 19.5c.7-3.4 3.4-5.3 6.5-5.3s5.8 1.9 6.5 5.3"/>',
  bolt: '<path d="M13.2 3.5L5.5 13.4h5.4l-1.1 7.1 7.7-9.9h-5.4l1.1-7.1z"/>',
  keyboard:
    '<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M8 14h8"/>',
  pin: '<path d="M9 4h6M12 4v7M12 11l4.5 3.5M12 11L7.5 14.5M12 20.5v-2"/>',
} as const;

export type IconName = keyof typeof ICONS;

export function Icon({ name, className }: { name: IconName; className?: string }) {
  return (
    <svg
      className={`ico${className ? " " + className : ""}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.65}
      strokeLinecap="round"
      strokeLinejoin="round"
      dangerouslySetInnerHTML={{ __html: ICONS[name] }}
    />
  );
}

/** 供需要原始 SVG 字符串的场景（如 dangerouslySetInnerHTML 构造菜单项）。 */
export const iconSvg = (name: IconName) =>
  `<svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;

/**
 * 品牌标记：与桌面应用图标同源的几何 N（斜画用半透明，带一点“动”的味道）。
 * 比例与应用图标一致——字形约占方块的一半高，颜色跟随 currentColor。
 */
export function BrandMark({ size = 20 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
      <g
        fill="none"
        stroke="currentColor"
        strokeWidth="2.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M6.2 15.2 13.8 4.8" opacity="0.5" />
        <path d="M6.2 4.8V15.2" />
        <path d="M13.8 4.8V15.2" />
      </g>
    </svg>
  );
}
