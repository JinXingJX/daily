import { useEffect, useState } from "react";

function nowSec() {
  return Math.floor(Date.now() / 1000);
}

/** Wall clock in seconds that only ticks while `live` is true, so idle views do not re-render. */
export function useNowSec(live: boolean) {
  const [now, setNow] = useState(nowSec);
  useEffect(() => {
    setNow(nowSec());
    if (!live) return;
    const timer = window.setInterval(() => setNow(nowSec()), 1000);
    return () => window.clearInterval(timer);
  }, [live]);
  return now;
}
