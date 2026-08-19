import { UserButton } from "@/lib/auth/gates";
import { useCurrentUserState } from "@/lib/auth/use-current-user";

export function AuthSlot() {
  const { user, isPending } = useCurrentUserState();

  if (isPending) {
    return <div className="size-8 animate-pulse rounded-full bg-elevated" />;
  }

  if (user) {
    return <UserButton />;
  }

  return (
    <a
      href="/login"
      className="inline-flex h-8 items-center rounded-md px-3 text-sm font-medium text-muted transition-colors duration-150 hover:bg-elevated hover:text-fg"
    >
      Sign in
    </a>
  );
}
