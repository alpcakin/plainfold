import { cookies } from "next/headers";
import { supabase } from "@/lib/db";

export type User = { id: string; email: string };

export async function getCurrentUser(): Promise<User | null> {
  const token = (await cookies()).get("sb-access-token")?.value;
  if (!token) return null;

  const { data, error } = await supabase.auth.getUser(token);
  if (error || !data.user) return null;
  if (!data.user.email_confirmed_at) return null;

  return { id: data.user.id, email: data.user.email ?? "" };
}
