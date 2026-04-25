import { useEffect, useState } from "react";
import { api, bytesToBlobUrl } from "../../ipc";

interface Props {
    assetId: number;
    size: number;
    mime: string;
    alt?: string;
    className?: string;
}

// Fetches the encrypted thumbnail via IPC, turns it into a blob URL. Revokes
// the URL on unmount or when `assetId` changes so memory doesn't leak.
export default function ThumbImage({ assetId, size, mime, alt, className }: Props) {
    const [url, setUrl] = useState<string | null>(null);
    const [failed, setFailed] = useState(false);

    useEffect(() => {
        let cancelled = false;
        let currentUrl: string | null = null;
        setFailed(false);
        void (async () => {
            try {
                const bytes = await api.assetThumbnail(assetId, size);
                if (cancelled) return;
                // Backend returns an empty byte array when no thumbnail is
                // cached yet (e.g. dev mock IPC, unindexed asset). Treat
                // that as a failure so the fallback renders instead of a
                // broken-image icon.
                if (bytes.length === 0) {
                    setFailed(true);
                    return;
                }
                // Thumbnails are always WebP regardless of source mime.
                currentUrl = bytesToBlobUrl(bytes, "image/webp");
                setUrl(currentUrl);
            } catch {
                if (!cancelled) setFailed(true);
            }
        })();
        return () => {
            cancelled = true;
            if (currentUrl) URL.revokeObjectURL(currentUrl);
        };
    }, [assetId, size]);

    if (failed) {
        return <div className={`thumb thumb-fallback ${className ?? ""}`}>{mime.split("/")[0]}</div>;
    }
    if (!url) {
        return <div className={`thumb thumb-loading ${className ?? ""}`} />;
    }
    return <img className={`thumb ${className ?? ""}`} src={url} alt={alt ?? ""} loading="lazy" />;
}
