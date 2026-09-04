import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { MessageAttachment } from "../../types/api";
import type { PendingImage } from "../../utils/imageAttachments";
import { Button, IconButton } from "../ui";

function ImagePreviewOverlay({
  url,
  alt,
  onClose,
}: {
  url: string;
  alt: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    document.body.style.overflow = "hidden";
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <div
      className="image-preview-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("Expand image")}
      onClick={onClose}
    >
      <IconButton
        label={t("Close")}
        size="sm"
        type="button"
        className="image-preview-close"
        onClick={onClose}
        title={t("Close")}
      >
        <X size={22} />
      </IconButton>
      <img src={url} alt={alt} onClick={(event) => event.stopPropagation()} />
    </div>,
    document.body,
  );
}

export function ImageAttachmentStrip({
  images,
  onRemove,
  disabled = false,
}: {
  images: PendingImage[];
  onRemove: (id: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const [previewId, setPreviewId] = useState<string | null>(null);
  const previewImage = images.find((image) => image.id === previewId);
  if (images.length === 0) return null;

  return (
    <>
      <div className="composer-image-attachments" aria-label={t("Pasted images")}>
        {images.map((image) => (
          <div className="composer-image-attachment" key={image.id}>
            <Button
              type="button"
              className="composer-image-attachment-preview"
              onClick={() => setPreviewId(image.id)}
              title={t("Expand image")}
              variant="ghost"
              size="sm"
            >
              <img src={image.previewUrl} alt={image.filename} />
            </Button>
            <IconButton
              label={t("Remove image")}
              size="sm"
              type="button"
              className="composer-image-attachment-remove"
              disabled={disabled}
              onClick={() => {
                setPreviewId((current) => (current === image.id ? null : current));
                onRemove(image.id);
              }}
              title={t("Remove image")}
            >
              <X size={18} />
            </IconButton>
          </div>
        ))}
      </div>
      {previewImage ? (
        <ImagePreviewOverlay
          url={previewImage.previewUrl}
          alt={previewImage.filename}
          onClose={() => setPreviewId(null)}
        />
      ) : null}
    </>
  );
}

function imageAttachments(attachments: MessageAttachment[]) {
  return attachments.filter(
    (attachment): attachment is Extract<MessageAttachment, { type: "image" }> =>
      attachment.type === "image" && attachment.data.length > 0,
  );
}

interface ImagePreview {
  image: Extract<MessageAttachment, { type: "image" }>;
  url: string;
}

function useImagePreviews(attachments: MessageAttachment[]): ImagePreview[] {
  const [previews, setPreviews] = useState<ImagePreview[]>([]);

  useEffect(() => {
    const next = imageAttachments(attachments).map((image) => ({
      image,
      url: URL.createObjectURL(new Blob([Uint8Array.from(image.data)], { type: image.mime })),
    }));
    setPreviews(next);

    return () => {
      for (const preview of next) URL.revokeObjectURL(preview.url);
    };
  }, [attachments]);

  return previews;
}

export function MessageImageGallery({ attachments }: { attachments: MessageAttachment[] }) {
  const { t } = useTranslation();
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const previews = useImagePreviews(attachments);

  const preview = previewIndex === null ? undefined : previews[previewIndex];
  if (previews.length === 0) return null;

  return (
    <>
      <div className="message-image-gallery">
        {previews.map(({ image, url }, index) => (
          <Button
            type="button"
            className="message-image-link"
            key={`${image.filename}:${image.file_size}:${index}`}
            onClick={() => setPreviewIndex(index)}
            title={t("Expand image")}
            variant="ghost"
            size="sm"
          >
            <img src={url} alt={image.filename} />
          </Button>
        ))}
      </div>
      {preview ? (
        <ImagePreviewOverlay
          url={preview.url}
          alt={preview.image.filename}
          onClose={() => setPreviewIndex(null)}
        />
      ) : null}
    </>
  );
}
