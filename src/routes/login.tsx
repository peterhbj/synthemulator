import { createFileRoute, Link } from "@tanstack/react-router";
import { GROK_PROVIDERS, authEnabled, signIn } from "@/lib/auth/client";
import { Button } from "@/components/ui/button";

export const Route = createFileRoute("/login")({ component: Login });

function GoogleMark() {
  return (
    <svg viewBox="0 0 24 24" className="size-4" aria-hidden>
      <path
        fill="currentColor"
        d="M21.35 11.1h-9.18v2.96h5.27c-.23 1.5-1.78 4.4-5.27 4.4-3.17 0-5.76-2.62-5.76-5.86s2.59-5.86 5.76-5.86c1.8 0 3.01.77 3.7 1.43l2.52-2.43C16.54 4.04 14.47 3.1 12.17 3.1 7.36 3.1 3.5 6.98 3.5 12.8s3.86 9.7 8.67 9.7c5 0 8.3-3.51 8.3-8.46 0-.57-.06-1-.12-1.94Z"
      />
    </svg>
  );
}

function XMark() {
  return (
    <svg viewBox="0 0 24 24" className="size-4" aria-hidden>
      <path
        fill="currentColor"
        d="M14.7 10.3 21.4 3h-1.6l-5.8 6.4L9.4 3H3.6l7 9.9L3.6 21h1.6l6.1-6.8 4.9 6.8h5.8l-7.3-10.7ZM12 13.3l-.7-1-5.6-7.8h2.4l4.5 6.4.7 1 5.9 8.2h-2.4L12 13.3Z"
      />
    </svg>
  );
}

function Login() {
  return (
    <main className="grid min-h-dvh place-items-center bg-bg px-6 text-fg">
      <div className="w-full max-w-sm rounded-3xl bg-surface p-6 shadow-[var(--shadow-panel)]">
        <p className="text-2xs font-medium uppercase tracking-[0.22em] text-subtle">
          Helix
        </p>
        <h1 className="mt-1 text-2xl font-semibold tracking-tight">Sign in</h1>
        <p className="mt-2 text-sm text-muted">
          Save your place. The keyboard still plays as a guest.
        </p>

        <div className="mt-6 space-y-2">
          {authEnabled ? (
            GROK_PROVIDERS.map((provider) => (
              <Button
                key={provider.providerId}
                type="button"
                variant="secondary"
                className="h-11 w-full justify-center"
                onClick={() => signIn(provider.providerId, { callbackURL: "/" })}
              >
                {provider.idp === "google" ? <GoogleMark /> : <XMark />}
                Continue with {provider.label}
              </Button>
            ))
          ) : (
            <p className="text-sm text-muted">Sign-in is disabled.</p>
          )}
        </div>

        <Link
          to="/"
          className="mt-6 inline-flex text-sm text-muted transition-colors duration-150 hover:text-fg"
        >
          Back to the keyboard
        </Link>
      </div>
    </main>
  );
}
