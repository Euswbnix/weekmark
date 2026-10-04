import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Outlet, useLocation } from "react-router";
import { useRefreshOnWindowFocus } from "@/api/queries";
import { brand } from "@/brand";
import { AccessoryBar } from "@/components/chrome/AccessoryBar";
import { CrashNotice } from "@/features/diagnostics/CrashNotice";
import { useAutoSync } from "@/features/sources/useAutoSync";
import { PostUpdateBanner } from "@/features/updates/PostUpdateBanner";
import { UpdateNotice } from "@/features/updates/UpdateNotice";
import { useUpdateLifecycle } from "@/features/updates/useUpdateLifecycle";
import { WhatsNewSheet } from "@/features/updates/WhatsNewSheet";
import { cn } from "@/lib/utils";
import { useRefreshAfterExternalSync } from "@/stores/sync";
import { LampContext } from "./lamp";
import { Sidebar } from "./Sidebar";
import { ToolbarContext, useScrolled } from "./toolbar";

/** The reading column (§3.0): layout.measure of text between layout.gutter margins. */
const READING_COLUMN =
  "mx-auto w-full max-w-[calc(var(--pl-layout-measure)+2*var(--pl-layout-gutter))] px-(--pl-layout-gutter)";

/**
 * Sidebar + scrollable content column. The column starts with a sticky toolbar row (a screen's
 * PageHeader puts its actions there), can light a lamp band behind its top (a section's `lit`)
 * and has the floating accessory bar at its foot. Every screen except onboarding renders
 * inside this.
 */
export function AppShell() {
  const mainRef = useRef<HTMLElement>(null);
  const [scroller, setScroller] = useState<HTMLElement | null>(null);
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  const setMain = useCallback((element: HTMLElement | null) => {
    mainRef.current = element;
    setScroller(element);
  }, []);
  const scrolled = useScrolled(scroller);
  const [lit, setLit] = useState(false);
  const toolbar = useMemo(() => (slot && scroller ? { slot, scroller } : null), [slot, scroller]);
  useRouteAnnouncements(mainRef);
  useRefreshAfterExternalSync();
  useRefreshOnWindowFocus();
  useUpdateLifecycle();
  useAutoSync();
  return (
    <div className="flex h-dvh overflow-hidden">
      <Sidebar />
      <div className="relative flex min-w-0 flex-1">
        <main
          ref={setMain}
          id="main"
          tabIndex={-1}
          className="pl-content relative isolate min-w-0 flex-1 overflow-y-auto outline-none"
        >
          {/* The lamp band's pool (§6.1): full-bleed, under the toolbar row, behind the text. */}
          <div aria-hidden className="pl-lamp pl-lamp-band" data-lit={lit || undefined} />
          {/* Glass only once content scrolls under it (§8). */}
          <div
            className={cn("pl-toolbar", scrolled && "pl-glass")}
            data-scrolled={scrolled || undefined}
          >
            <div ref={setSlot} className={cn(READING_COLUMN, "flex h-full items-center gap-4")} />
          </div>
          <LampContext.Provider value={setLit}>
            <ToolbarContext.Provider value={toolbar}>
              {/* Room at the foot for the accessory bar. */}
              <div className={cn(READING_COLUMN, "pt-2 pb-20")}>
                <CrashNotice />
                <PostUpdateBanner />
                <UpdateNotice />
                <WhatsNewSheet />
                <Outlet />
              </div>
            </ToolbarContext.Provider>
          </LampContext.Provider>
        </main>
        <AccessoryBar />
      </div>
    </div>
  );
}

/**
 * After a navigation (not on first load): if the click that navigated left focus nowhere, move
 * it to the page so keyboard users continue from the top, and name the window after the page's
 * heading once it has rendered.
 */
function useRouteAnnouncements(mainRef: React.RefObject<HTMLElement | null>) {
  const { pathname } = useLocation();
  const lastPath = useRef<string | null>(null);
  useEffect(() => {
    const main = mainRef.current;
    if (!main) return;
    const navigated = lastPath.current !== null && lastPath.current !== pathname;
    lastPath.current = pathname;
    if (navigated && (document.activeElement === document.body || !document.activeElement)) {
      main.focus({ preventScroll: true });
    }
    main.scrollTop = 0;
    // The h1 may appear after data loads; watch for it and keep the title in sync.
    const update = () => {
      const heading = main.querySelector("h1")?.textContent?.trim();
      document.title = heading ? `${heading} – ${brand.productName}` : brand.productName;
    };
    update();
    const observer = new MutationObserver(update);
    observer.observe(main, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [pathname, mainRef]);
}
