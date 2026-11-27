export type Locale = 'en' | 'zh';

export interface Feature {
  title: string;
  description: string;
}

export interface ProductCopy {
  title: string;
  description: string;
  eyebrow: string;
  headline: [string, string];
  intro: string;
  imageAlt: string;
  imageCaption: string;
  highlights: Feature[];
  story: {
    label: string;
    title: string;
    paragraphs: string[];
    quote?: string;
    quoteLabel?: string;
    imageAlt?: string;
    imageCaption?: string;
  };
  featuresTitle: string;
  features: Feature[];
  detail: { title: string; description: string; items: string[]; note: string };
  stepsTitle: string;
  steps: Feature[];
  faqTitle: string;
  faq: { question: string; answer: string }[];
  downloadTitle: string;
  downloadDescription: string;
}

export interface Product {
  id: 'musicx' | 'folio';
  name: string;
  repository: string;
  guide: string;
  release?: {
    tag: string;
    url: string;
    download: string;
  };
  hero: { src: string; width: number; height: number };
  storyImage?: { src: string; width: number; height: number };
  copy: Record<Locale, ProductCopy>;
}
