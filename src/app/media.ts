import manifest from '../../public/media/manifest.json';

function localAsset(value: unknown): string | null {
  return typeof value === 'string' && /^\/media\/[a-zA-Z0-9_-]+\.(webp|png|jpg|mp4)$/.test(value) ? value : null;
}

export const media = {
  cozy: localAsset(manifest.images.cozy),
  coast: localAsset(manifest.images.coast),
  ambient: localAsset(manifest.video.ambient),
  poster: localAsset(manifest.video.poster) ?? localAsset(manifest.images.cozy),
};
