import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/seo";
import { postsByDate } from "@/lib/blog";
import { DOCS } from "@/lib/docs";

export default function sitemap(): MetadataRoute.Sitemap {
  const lastBlog = postsByDate[0]?.date ?? new Date().toISOString();

  return [
    {
      url: SITE_URL,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 1,
    },
    {
      url: `${SITE_URL}/hub`,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 0.8,
    },
    {
      url: `${SITE_URL}/docs`,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 0.9,
    },
    ...DOCS.map((doc) => ({
      url: `${SITE_URL}/docs/${doc.slug}`,
      lastModified: new Date(),
      changeFrequency: "monthly" as const,
      priority: 0.7,
    })),
    {
      url: `${SITE_URL}/blog`,
      lastModified: new Date(lastBlog),
      changeFrequency: "weekly",
      priority: 0.7,
    },
    ...postsByDate.map((post) => ({
      url: `${SITE_URL}/blog/${post.slug}`,
      lastModified: new Date(post.date),
      changeFrequency: "monthly" as const,
      priority: 0.6,
      // Image sitemap entry — surfaces the cover in Google Images.
      images: [`${SITE_URL}${post.cover}`],
    })),
  ];
}
