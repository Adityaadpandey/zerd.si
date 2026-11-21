import { Brand } from "./brand";

// Which AI models zerd can drive.
export default function Models() {
  return (
    <ul className="models" aria-label="Works with">
      <li><Brand name="claude" />Claude</li>
      <li><Brand name="openai" />GPT and Codex</li>
      <li><Brand name="deepseek" />DeepSeek</li>
      <li><Brand name="minimax" />MiniMax</li>
      <li><Brand name="ollama" />Local models</li>
    </ul>
  );
}
