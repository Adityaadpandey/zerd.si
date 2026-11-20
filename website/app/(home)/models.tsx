import { BrandIcon } from "./icons";

// Which models zerd can drive. The marks are the providers' own; GPT has no mark here, so it is a name.
export default function Models() {
  return (
    <ul className="models" aria-label="Works with">
      <li><BrandIcon name="claude" />Claude</li>
      <li>GPT and Codex</li>
      <li><BrandIcon name="deepseek" />DeepSeek</li>
      <li><BrandIcon name="ollama" />Local models</li>
    </ul>
  );
}
