import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import SyntaxHighlighter from "react-syntax-highlighter/dist/esm/prism-light";
import bash from "react-syntax-highlighter/dist/esm/languages/prism/bash";
import diff from "react-syntax-highlighter/dist/esm/languages/prism/diff";
import javascript from "react-syntax-highlighter/dist/esm/languages/prism/javascript";
import json from "react-syntax-highlighter/dist/esm/languages/prism/json";
import jsx from "react-syntax-highlighter/dist/esm/languages/prism/jsx";
import markdown from "react-syntax-highlighter/dist/esm/languages/prism/markdown";
import python from "react-syntax-highlighter/dist/esm/languages/prism/python";
import rust from "react-syntax-highlighter/dist/esm/languages/prism/rust";
import tsx from "react-syntax-highlighter/dist/esm/languages/prism/tsx";
import typescript from "react-syntax-highlighter/dist/esm/languages/prism/typescript";
import "katex/dist/katex.min.css";
import { renderPluginBlock } from "@/features/plugins/inline/registry";

type ChatRendererProps = {
  content: string;
  isStreaming?: boolean;
};

type PartialPluginFence = {
  stableContent: string;
  identifier: string;
  payload: string;
};

SyntaxHighlighter.registerLanguage("bash", bash);
SyntaxHighlighter.registerLanguage("diff", diff);
SyntaxHighlighter.registerLanguage("javascript", javascript);
SyntaxHighlighter.registerLanguage("json", json);
SyntaxHighlighter.registerLanguage("jsx", jsx);
SyntaxHighlighter.registerLanguage("markdown", markdown);
SyntaxHighlighter.registerLanguage("python", python);
SyntaxHighlighter.registerLanguage("rust", rust);
SyntaxHighlighter.registerLanguage("tsx", tsx);
SyntaxHighlighter.registerLanguage("typescript", typescript);

const codeTheme = {
  'code[class*="language-"]': {
    color: "var(--text-strong)",
    background: "transparent",
    fontFamily: "var(--font-mono)",
    fontSize: "13px",
    lineHeight: 1.6,
    textAlign: "left" as const,
    whiteSpace: "pre" as const,
    wordSpacing: "normal",
    wordBreak: "normal" as const,
    wordWrap: "normal" as const,
    tabSize: 2,
    hyphens: "none" as const,
  },
  'pre[class*="language-"]': {
    color: "var(--text-strong)",
    background: "var(--surface)",
    fontFamily: "var(--font-mono)",
    fontSize: "13px",
    lineHeight: 1.6,
    margin: 0,
    padding: "1rem",
    overflow: "auto",
  },
  comment: { color: "var(--text-muted)" },
  prolog: { color: "var(--text-muted)" },
  doctype: { color: "var(--text-muted)" },
  cdata: { color: "var(--text-muted)" },
  punctuation: { color: "var(--text-soft)" },
  property: { color: "var(--danger)" },
  tag: { color: "var(--danger)" },
  boolean: { color: "var(--danger)" },
  number: { color: "var(--danger)" },
  constant: { color: "var(--danger)" },
  symbol: { color: "var(--danger)" },
  deleted: { color: "var(--danger)" },
  selector: { color: "var(--success)" },
  "attr-name": { color: "var(--success)" },
  string: { color: "var(--success)" },
  char: { color: "var(--success)" },
  builtin: { color: "var(--success)" },
  inserted: { color: "var(--success)" },
  operator: { color: "var(--warning)" },
  entity: { color: "var(--warning)" },
  url: { color: "var(--warning)" },
  atrule: { color: "var(--accent)" },
  "attr-value": { color: "var(--accent)" },
  keyword: { color: "var(--accent)" },
  function: { color: "var(--info)" },
  "class-name": { color: "var(--info)" },
  regex: { color: "var(--warning)" },
  important: { color: "var(--warning)" },
  variable: { color: "var(--warning)" },
};

export function ChatRenderer({ content, isStreaming }: ChatRendererProps) {
  const partialFence = isStreaming ? findPartialPluginFence(content) : null;
  const stableContent = partialFence ? partialFence.stableContent : content;

  return (
    <div className="reading space-y-3">
      {stableContent.trim() ? <MarkdownContent content={stableContent} /> : null}
      {partialFence ? renderPluginBlock(partialFence.identifier, partialFence.payload, true).content : null}
    </div>
  );
}

function MarkdownContent({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={[rehypeKatex]}
      components={{
        h1: ({ children }) => <h1 className="pt-2 text-[24px] font-semibold leading-8">{children}</h1>,
        h2: ({ children }) => <h2 className="pt-2 text-[20px] font-semibold leading-7">{children}</h2>,
        h3: ({ children }) => <h3 className="pt-1 text-[17px] font-semibold leading-7">{children}</h3>,
        p: ({ children }) => <p>{children}</p>,
        ul: ({ children }) => <ul className="ml-5 list-disc space-y-1 marker:text-[var(--text-muted)]">{children}</ul>,
        ol: ({ children }) => <ol className="ml-5 list-decimal space-y-1 marker:text-[var(--text-muted)]">{children}</ol>,
        li: ({ children }) => <li className="pl-1">{children}</li>,
        a: ({ children, href }) => (
          <a className="text-[var(--accent-text)] underline decoration-[var(--accent-line)] underline-offset-4 hover:decoration-current" href={href}>
            {children}
          </a>
        ),
        blockquote: ({ children }) => (
          <blockquote className="border-l-2 border-[var(--line-strong)] pl-4 text-[var(--text-soft)]">{children}</blockquote>
        ),
        table: ({ children }) => (
          <div className="overflow-x-auto rounded-[var(--radius-panel)] border border-[var(--line)] font-sans">
            <table className="min-w-full border-collapse text-[14px] leading-5">{children}</table>
          </div>
        ),
        th: ({ children }) => (
          <th className="border-b border-[var(--line)] bg-[var(--surface)] px-4 py-2.5 text-left font-medium text-[var(--text-strong)]">
            {children}
          </th>
        ),
        td: ({ children }) => <td className="border-b border-[var(--line-soft)] px-4 py-2.5 text-[var(--text)]">{children}</td>,
        hr: () => <div className="my-4 h-px w-full bg-[var(--line)]" />,
        code({ className, children, ...props }) {
          const language = className?.replace("language-", "") ?? "";
          const source = String(children).replace(/\n$/, "");

          if (language === "daily-plugin" || language.startsWith("plugin:")) {
            return renderPluginBlock(language, source, false).content;
          }

          if (!className && !source.includes("\n")) {
            return (
              <code
                {...props}
                className="rounded-[6px] bg-[var(--code-bg)] px-1.5 py-0.5 font-mono text-[13px] text-[var(--text-strong)]"
              >
                {children}
              </code>
            );
          }

          return (
            <SyntaxHighlighter
              style={codeTheme}
              language={language || "text"}
              PreTag="div"
              customStyle={{
                margin: 0,
                borderRadius: 12,
                background: "var(--surface)",
                padding: "14px 16px",
                border: "1px solid var(--line)",
              }}
            >
              {source}
            </SyntaxHighlighter>
          );
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
}

function findPartialPluginFence(content: string): PartialPluginFence | null {
  const dailyOpenIndex = content.lastIndexOf("```daily-plugin");
  const legacyOpenIndex = content.lastIndexOf("```plugin:");
  const openIndex = Math.max(dailyOpenIndex, legacyOpenIndex);
  if (openIndex === -1) {
    return null;
  }

  const candidate = content.slice(openIndex);
  const infoLineBreak = candidate.indexOf("\n");
  if (infoLineBreak === -1) {
    return {
      stableContent: content.slice(0, openIndex),
      identifier: candidate.replace("```", "").trim(),
      payload: "",
    };
  }

  const infoLine = candidate.slice(3, infoLineBreak).trim();
  const remainder = candidate.slice(infoLineBreak + 1);
  if (remainder.includes("\n```") || remainder.endsWith("```")) {
    return null;
  }

  return {
    stableContent: content.slice(0, openIndex),
    identifier: infoLine,
    payload: remainder,
  };
}
