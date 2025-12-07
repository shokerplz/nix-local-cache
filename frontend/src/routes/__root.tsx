import { createRootRoute, Link, Outlet } from '@tanstack/react-router'
import { TanStackRouterDevtools } from '@tanstack/router-devtools'
import { useEffect, useState } from 'react'

export const Route = createRootRoute({
  component: () => {
    const [isDark, setIsDark] = useState(true)

    useEffect(() => {
      const root = window.document.documentElement
      if (isDark) {
        root.classList.add('dark')
      } else {
        root.classList.remove('dark')
      }
    }, [isDark])

    return (
      <>
        <div className="min-h-screen bg-background font-sans antialiased">
          <header className="sticky top-0 z-50 w-full border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
            <div className="container flex h-14 items-center m-auto">
              <div className="mr-4 flex">
                <Link to="/" className="mr-6 flex items-center space-x-2">
                  <span className="font-bold">Nix Cache</span>
                </Link>
                <nav className="flex items-center space-x-6 text-sm font-medium">
                  <Link
                    to="/"
                    className="transition-colors hover:text-foreground/80 text-foreground/60"
                    activeProps={{ className: '!text-foreground' }}
                  >
                    Dashboard
                  </Link>
                  <Link
                    to="/build/new"
                    className="transition-colors hover:text-foreground/80 text-foreground/60"
                    activeProps={{ className: '!text-foreground' }}
                  >
                    New Build
                  </Link>
                </nav>
              </div>
              <div className="flex flex-1 items-center justify-between space-x-2 md:justify-end">
                <button
                  onClick={() => setIsDark(!isDark)}
                  className="inline-flex items-center justify-center whitespace-nowrap rounded-md text-sm font-medium ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 border border-input bg-background hover:bg-accent hover:text-accent-foreground h-9 w-9"
                >
                  {isDark ? (
                    <svg xmlns="http://www.w3.org/2000/svg" width="1.2rem" height="1.2rem" viewBox="0 0 24 24"><path fill="currentColor" d="M12 7c-2.76 0-5 2.24-5 5s2.24 5 5 5s5-2.24 5-5s-2.24-5-5-5M2 13h2c.55 0 1-.45 1-1s-.45-1-1-1H2c-.55 0-1 .45-1 1s.45 1 1 1m18 0h2c.55 0 1-.45 1-1s-.45-1-1-1h-2c-.55 0-1 .45-1 1s.45 1 1 1M11 2v2c0 .55.45 1 1 1s1-.45 1-1V2c0-.55-.45-1-1-1s-1 .45-1 1m0 18v2c0 .55.45 1 1 1s1-.45 1-1v-2c0-.55-.45-1-1-1s-1 .45-1 1M5.99 4.58a.996.996 0 0 0-1.41 0a.996.996 0 0 0 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0s.39-1.03 0-1.41zm12.37 12.37a.996.996 0 0 0-1.41 0a.996.996 0 0 0 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0a.996.996 0 0 0 0-1.41zm1.06-10.96a.996.996 0 0 0 0-1.41a.996.996 0 0 0-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0zM7.05 18.36a.996.996 0 0 0 0 1.41a.996.996 0 0 0 1.41 0l1.06-1.06c.39-.39.39-1.03 0-1.41s-1.03-.39-1.41 0z"/></svg>
                  ) : (
                    <svg xmlns="http://www.w3.org/2000/svg" width="1.2rem" height="1.2rem" viewBox="0 0 24 24"><path fill="currentColor" d="M12 3a9 9 0 1 0 9 9c0-.46-.04-.92-.1-1.36a5.389 5.389 0 0 1-4.4 2.26a5.403 5.403 0 0 1-3.14-9.8c-.44-.06-.9-.1-1.36-.1"/></svg>
                  )}
                  <span className="sr-only">Toggle theme</span>
                </button>
              </div>
            </div>
          </header>
          <main className="container py-6 m-auto">
            <Outlet className="container m-auto"/>
          </main>
        </div>
        <TanStackRouterDevtools />
      </>
    )
  },
})
