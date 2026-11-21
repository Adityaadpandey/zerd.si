// Provider marks, served as files from /public/logos so they stay out of the JS bundle.
const FILES = {
  claude: "claude-color.svg", claudecode: "claudecode-color.svg", openai: "openai.svg",
  deepseek: "deepseek-color.svg", ollama: "ollama.svg", minimax: "minimax-color.svg",
} as const;
export type BrandName = keyof typeof FILES;

export function Brand({ name, size = 18 }: { name: BrandName; size?: number }) {
  // eslint-disable-next-line @next/next/no-img-element
  return <img className="brand-ic" src={`/logos/${FILES[name]}`} alt="" width={size} height={size} draggable={false} />;
}
