import type { ReactNode } from "react";

const line = (children: ReactNode, size: number) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">{children}</svg>
);

export const Check = () => line(<path d="m5 12.5 4.5 4.5L19 7.5" />, 18);
export const Cross = () => line(<path d="M6 6l12 12M18 6 6 18" />, 16);
export const Arrow = () => line(<path d="M7 17 17 7M8 7h9v9" />, 14);
