import { MarkdownRenderer } from "../renderers/MarkdownRenderer";

interface MarkdownPreviewProps {
  content: string;
}

export function MarkdownPreview({ content }: MarkdownPreviewProps) {
  return (
    <div className="h-full overflow-auto p-4">
      <MarkdownRenderer content={content} />
    </div>
  );
}
