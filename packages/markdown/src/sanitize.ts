import DOMPurify, { type Config, type WindowLike } from 'dompurify';

/**
 * The second sanitization layer (the first is markdown-it with `html: false`,
 * which escapes raw HTML). An explicit allowlist rather than DOMPurify's
 * profiles: the preview renders documents, not applications.
 */
export interface Sanitizer {
  sanitize(html: string): string;
}

export const ALLOWED_TAGS: readonly string[] = [
  'p',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'a',
  'img',
  'ul',
  'ol',
  'li',
  'table',
  'thead',
  'tbody',
  'tr',
  'th',
  'td',
  'pre',
  'code',
  'em',
  'strong',
  's',
  'del',
  'blockquote',
  'hr',
  'br',
  'input',
  'span',
  'div',
  'sup',
  'sub',
];

/** `data-*` attributes are allowed by DOMPurify's default `ALLOW_DATA_ATTR`. */
export const ALLOWED_ATTR: readonly string[] = [
  'href',
  'src',
  'alt',
  'title',
  'class',
  'id',
  'align',
  'start',
  'type',
  'checked',
  'disabled',
];

/**
 * `http(s):`, fragments and relative paths. No `mailto:` (the core opens
 * http(s) only), no other scheme. `data:` never matches here; DOMPurify lets
 * it through for `<img>` only, and the hook below restricts the MIME type.
 */
export const ALLOWED_URI_REGEXP = /^(?:https?:|[^a-z]|[a-z+.-]+(?:[^a-z+.:-]|$))/i;

const IMAGE_DATA_URI = /^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/=]+$/i;
/** Inline images larger than this are dropped (bytes of the data URI). */
export const MAX_IMAGE_DATA_URI_LENGTH = 2 * 1024 * 1024;
/** Only ids the renderer minted, so a document cannot clobber the app's DOM ids. */
export const ID_PREFIX = 'md-';

const CONFIG: Config = {
  ALLOWED_TAGS: [...ALLOWED_TAGS],
  ALLOWED_ATTR: [...ALLOWED_ATTR],
  ALLOWED_URI_REGEXP,
  ALLOW_DATA_ATTR: true,
  KEEP_CONTENT: true,
};

/**
 * Bind DOMPurify to a window. Throws when DOMPurify cannot run there: without
 * a DOM it returns its input untouched, which must never pass for sanitized.
 */
export function createSanitizer(window: WindowLike): Sanitizer {
  const purify = DOMPurify(window);
  if (!purify.isSupported) {
    throw new Error('DOMPurify is not supported in this environment (no DOM available)');
  }

  purify.addHook('uponSanitizeElement', (node, data) => {
    if (data.tagName !== 'input') return;
    const element = node as Element;
    const isCheckbox =
      element.getAttribute('type')?.toLowerCase() === 'checkbox' &&
      element.hasAttribute('disabled');
    if (!isCheckbox) element.parentNode?.removeChild(element);
  });

  purify.addHook('uponSanitizeAttribute', (node, data) => {
    const tag = node.nodeName.toLowerCase();
    switch (data.attrName) {
      case 'src':
        // Only inline images: the WebView cannot load repository files and
        // must not fetch anything else (CSP: `img-src 'self' data: blob:`).
        data.keepAttr =
          tag === 'img' &&
          data.attrValue.length <= MAX_IMAGE_DATA_URI_LENGTH &&
          IMAGE_DATA_URI.test(data.attrValue);
        return;
      case 'id':
        data.keepAttr = data.attrValue.startsWith(ID_PREFIX);
        return;
      case 'type':
        data.keepAttr = tag === 'input' && data.attrValue.toLowerCase() === 'checkbox';
        return;
      case 'checked':
      case 'disabled':
        data.keepAttr = tag === 'input';
        return;
      case 'align':
        data.keepAttr =
          (tag === 'th' || tag === 'td') && /^(?:left|center|right)$/i.test(data.attrValue);
        return;
      case 'start':
        data.keepAttr = tag === 'ol' && /^\d{1,9}$/.test(data.attrValue);
        return;
      default:
        return;
    }
  });

  return {
    sanitize: (html) => purify.sanitize(html, CONFIG),
  };
}
