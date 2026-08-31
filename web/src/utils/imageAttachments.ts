import type { PromptImageAttachment } from "../types/api";

export interface PendingImage {
  id: string;
  file: File;
  filename: string;
  mime: string;
  fileSize: number;
  previewUrl: string;
}

export function pastedImageFiles(clipboardData: DataTransfer): File[] {
  return Array.from(clipboardData.items)
    .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
}

export function createPendingImages(files: File[]): PendingImage[] {
  return files.map((file, index) => ({
    id: crypto.randomUUID(),
    file,
    filename: file.name || `clipboard-image-${index + 1}.png`,
    mime: file.type || "image/png",
    fileSize: file.size,
    previewUrl: URL.createObjectURL(file),
  }));
}

export async function pendingImagesToPromptAttachments(
  images: PendingImage[],
): Promise<PromptImageAttachment[]> {
  return Promise.all(
    images.map(async (image) => ({
      type: "image" as const,
      filename: image.filename,
      mime: image.mime,
      data: Array.from(new Uint8Array(await image.file.arrayBuffer())),
    })),
  );
}

export function formatImageFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
