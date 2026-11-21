import Link from "next/link";
import Logo from "./logo";
import InstallButton from "./install-button";

// A floating glass bar over the page.
export default function Nav({ repo }: { repo: string }) {
  return (
    <header className="nav">
      <div className="nav-wrap">
        <div className="nav-pill">
          <Logo />
          <nav aria-label="Main">
            <Link href="/how-it-works">How it works</Link>
            <Link href="/proof">Proof</Link>
            <Link href="/docs/quickstart">Docs</Link>
            <a href={`https://github.com/${repo}`}>GitHub</a>
          </nav>
          <InstallButton />
        </div>
      </div>
    </header>
  );
}
