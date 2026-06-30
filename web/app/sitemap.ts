import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/seo";
import { postsByDate } from "@/lib/blog";

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
    })),
  ];
}
