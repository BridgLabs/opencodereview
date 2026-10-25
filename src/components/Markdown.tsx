import type { ReactNode } from "react";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import remarkGfm from "remark-gfm";
import { openExternal } from "../lib/openExternal";

/// Renders GitHub-flavored markdown (tables, code fences, lists) plus raw HTML
/// like `<details>/<summary>` collapsibles, matching how GitHub shows reviews.
///
/// All links open in the OS browser (not the in-app webview, which would lose the
/// GitHub session and show a 404). `imagesAsLinks` renders images as a compact
/// link instead of an `<img>` — used in conversations, where GitHub attachment
/// images can't load in the webview and would otherwise show a broken box.
export function Markdown({ children, imagesAsLinks = false }: { children: string; imagesAsLinks?: boolean }) {
  const components: Components = {
    a: (props) => {
      const { href, children: kids } = props as { href?: string; children?: ReactNode };
      return (
        <a
          href={href}
          onClick={(event) => {
            if (href) {
              event.preventDefault();
              void openExternal(href);
            }
          }}
        >
          {kids}
        </a>
      );
    },
    ...(imagesAsLinks
      ? {
          img: (props) => {
            const { src, alt } = props as { src?: string; alt?: string };
            if (!src) return null;
            return (
              <a
                href={src}
                className="md-image-link"
                onClick={(event) => {
                  event.preventDefault();
                  void openExternal(src);
                }}
              >
                🖼 {alt && alt !== "image" ? alt : "image"}
              </a>
            );
          },
        }
      : {}),
  };
  return (
    <div className="markdown">
      <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeRaw]} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
