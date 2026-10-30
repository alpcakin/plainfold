import { createClient } from "npm:@supabase/supabase-js@2";

Deno.serve(async () => {
  const supabase = createClient(
    Deno.env.get("SUPABASE_URL")!,
    Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!
  );

  const cutoff = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
  const { data, error } = await supabase
    .from("books")
    .delete()
    .lt("deleted_at", cutoff)
    .select("id");

  if (error) return new Response(error.message, { status: 500 });

  await fetch(Deno.env.get("MAIL_API_URL")!, {
    method: "POST",
    body: JSON.stringify({ subject: "Nightly cleanup", removed: data.length }),
  });

  return new Response(JSON.stringify({ removed: data.length }));
});
