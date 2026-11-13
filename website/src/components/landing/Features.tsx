"use client";

import { useI18n } from "@/i18n/provider";

export default function Features() {
  const { t } = useI18n();

  return (
    <section id="features" className="py-section px-6">
      <div className="mx-auto max-w-6xl">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary text-center">
          {t.features.title}
        </h2>
        <p className="mt-3 text-text-secondary text-center max-w-xl mx-auto text-[15px] leading-relaxed">
          {t.features.subtitle}
        </p>

        <div className="mt-14 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-px bg-border rounded-lg overflow-hidden">
          {t.features.items.map((item) => (
            <div key={item.title} className="bg-surface p-6">
              <h3 className="text-sm font-semibold text-text-primary mb-2">
                {item.title}
              </h3>
              <p className="text-[13px] leading-relaxed text-text-secondary">
                {item.desc}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
