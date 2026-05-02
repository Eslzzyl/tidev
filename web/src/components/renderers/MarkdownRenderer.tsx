import ReactMarkdown from 'react-markdown';
import rehypeHighlight from 'rehype-highlight';
import type { ComponentProps } from 'react';

interface Props {
  content: string;
}

/**
 * Custom link component that opens in a new tab.
 */
function CustomLink(props: ComponentProps<'a'>) {
  return (
    <a
      {...props}
      target="_blank"
      rel="noopener noreferrer"
      className="text-blue-600 hover:text-blue-800 underline dark:text-blue-400 dark:hover:text-blue-300"
    />
  );
}

export function MarkdownRenderer({ content }: Props) {
  if (!content) return null;

  return (
    <div className="markdown-body prose prose-sm dark:prose-invert max-w-none">
      <ReactMarkdown
        rehypePlugins={[rehypeHighlight]}
        components={{
          a: CustomLink,
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
