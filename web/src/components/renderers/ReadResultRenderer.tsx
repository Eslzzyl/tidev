import { File, Folder } from "lucide-react";

import type { MessageAttachment } from "../../types/api";
import { MessageImageGallery } from "../chat/ImageAttachments";
import { CodeLinesRenderer } from "./CodeLinesRenderer";

type ReadResultKind = "text" | "directory" | "image";

export interface DirectoryReadEntry {
  name: string;
  isDirectory: boolean;
}

export interface DirectoryReadOutput {
  path: string;
  entries: DirectoryReadEntry[];
}

function directoryAttachment(attachments: MessageAttachment[]) {
  return attachments.find(
    (attachment): attachment is Extract<MessageAttachment, { type: "directory_reference" }> =>
      attachment.type === "directory_reference",
  );
}

function imageAttachments(attachments: MessageAttachment[]) {
  return attachments.filter(
    (attachment): attachment is Extract<MessageAttachment, { type: "image" }> =>
      attachment.type === "image" && attachment.data.length > 0,
  );
}

export function readResultKind(attachments: MessageAttachment[]): ReadResultKind {
  if (attachments.some((attachment) => attachment.type === "image")) return "image";
  if (attachments.some((attachment) => attachment.type === "directory_reference")) {
    return "directory";
  }
  return "text";
}

export function parseDirectoryReadOutput(output: string, attachmentPath = ""): DirectoryReadOutput {
  const directoryOutput = output.split("\n\n<system-reminder>", 1)[0] ?? "";
  const lines = directoryOutput
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const isEmpty = lines.length === 1 && lines[0] === "(empty)";
  const listedEntries = isEmpty ? [] : lines.slice(1);
  const entries = listedEntries
    .map((name) => ({ name, isDirectory: name.endsWith("/") }))
    .sort(
      (left, right) =>
        Number(right.isDirectory) - Number(left.isDirectory) || left.name.localeCompare(right.name),
    );

  return {
    path: attachmentPath || (isEmpty ? "" : (lines[0] ?? "")),
    entries,
  };
}

export function formatFileSize(bytes: number) {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

function ReadTextRenderer({ output, filepath }: { output: string; filepath?: string }) {
  return (
    <section className="tool-read-result tool-read-text">
      <CodeLinesRenderer output={output} filepath={filepath} />
    </section>
  );
}

function DirectoryReadRenderer({
  output,
  attachments,
}: {
  output: string;
  attachments: MessageAttachment[];
}) {
  const attachment = directoryAttachment(attachments);
  const directory = parseDirectoryReadOutput(output, attachment?.path);

  return (
    <section className="tool-read-result tool-read-directory">
      {directory.entries.length > 0 ? (
        <ul className="tool-directory-list">
          {directory.entries.map((entry) => (
            <li className="tool-directory-entry" key={entry.name}>
              {entry.isDirectory ? (
                <Folder size={15} aria-hidden="true" />
              ) : (
                <File size={15} aria-hidden="true" />
              )}
              <span>{entry.name}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="tool-directory-empty">(empty)</p>
      )}
    </section>
  );
}

function ImageReadRenderer({
  attachments,
  output,
}: {
  attachments: MessageAttachment[];
  output: string;
}) {
  const images = imageAttachments(attachments);
  const image = images[0];

  if (!image) return <pre className="tool-code-lines-fallback">{output}</pre>;

  return (
    <section className="tool-read-result tool-read-image">
      <MessageImageGallery attachments={images} />
    </section>
  );
}

export function ReadResultRenderer({
  output,
  filepath,
  attachments,
}: {
  output: string;
  filepath?: string;
  attachments: MessageAttachment[];
}) {
  switch (readResultKind(attachments)) {
    case "directory":
      return <DirectoryReadRenderer output={output} attachments={attachments} />;
    case "image":
      return <ImageReadRenderer output={output} attachments={attachments} />;
    default:
      return <ReadTextRenderer output={output} filepath={filepath} />;
  }
}
