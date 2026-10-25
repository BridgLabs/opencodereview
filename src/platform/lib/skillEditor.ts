import { Node } from "@tiptap/core";
import CodeBlock from "@tiptap/extension-code-block";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { TableKit } from "@tiptap/extension-table";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";

const RawMarkdown = Node.create({
  name: "rawMarkdownInline",
  group: "inline",
  inline: true,
  atom: true,
  addAttributes: () => ({ raw: { default: "" } }),
  parseHTML: () => [{ tag: "span[data-raw-markdown]", getAttrs: (element) => ({ raw: element.textContent }) }],
  renderHTML: ({ node }) => ["span", { "data-raw-markdown": "", class: "skill-raw-markdown" }, node.attrs.raw],
  markdownTokenName: "html",
  parseMarkdown: (token) => (token.block ? [] : { type: "rawMarkdownInline", attrs: { raw: token.raw } }),
  renderMarkdown: (node) => node.attrs?.raw ?? "",
});

const RawMarkdownInline = RawMarkdown.extend({
  markdownTokenName: "rawMarkdownInline",
  markdownTokenizer: {
    name: "rawMarkdownInline",
    level: "inline",
    start: (source) => source.indexOf("<"),
    tokenize: (source) => {
      const raw = source.match(/^(?:<!--[\s\S]*?-->|<\/?[a-zA-Z][\w-]*(?:\s[^>]*|\s*\/?)>)/)?.[0];
      return raw ? { type: "rawMarkdownInline", raw } : undefined;
    },
  },
});

const RawMarkdownBlock = RawMarkdown.extend({
  name: "rawMarkdownBlock",
  group: "block",
  inline: false,
  parseHTML: () => [{ tag: "pre[data-raw-markdown]", getAttrs: (element) => ({ raw: element.textContent }) }],
  renderHTML: ({ node }) => ["pre", { "data-raw-markdown": "", class: "skill-raw-markdown" }, node.attrs.raw],
  parseMarkdown: (token) =>
    token.block ? { type: "rawMarkdownBlock", attrs: { raw: token.raw?.trimEnd() ?? "" } } : [],
});

const MarkdownImage = RawMarkdown.extend({
  name: "markdownImage",
  markdownTokenName: "image",
  parseHTML: () => [{ tag: "span[data-markdown-image]", getAttrs: (element) => ({ raw: element.textContent }) }],
  renderHTML: ({ node }) => ["span", { "data-markdown-image": "", class: "skill-raw-markdown" }, node.attrs.raw],
  parseMarkdown: (token) => ({ type: "markdownImage", attrs: { raw: token.raw } }),
});

export const skillEditorExtensions = [
  StarterKit.configure({
    link: { openOnClick: false, autolink: false },
    underline: false,
    trailingNode: false,
    codeBlock: false,
  }),
  CodeBlock.extend({
    renderMarkdown: (node) => {
      const code = node.content?.map((child) => child.text ?? "").join("") ?? "";
      const longest = (code.match(/`{3,}/g) ?? []).reduce((length, run) => Math.max(length, run.length), 2);
      const fence = "`".repeat(longest + 1);
      return `${fence}${node.attrs?.language ?? ""}\n${code}\n${fence}`;
    },
  }),
  TableKit.configure({ table: { resizable: false } }),
  TaskList,
  TaskItem.configure({ nested: true }),
  RawMarkdownInline,
  RawMarkdownBlock,
  MarkdownImage,
  Markdown,
];
