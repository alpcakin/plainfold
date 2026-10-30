import { NextResponse } from "next/server";
import { getCurrentUser } from "@/lib/auth";
import { supabase } from "@/lib/db";
import { coverUrlFor } from "@/lib/covers";
import type { Book } from "@/types/book";

const PAGE_SIZE = 20;

export async function GET(request: Request) {
  const user = await getCurrentUser();
  if (!user) {
    return NextResponse.json({ error: "Not logged in" }, { status: 401 });
  }

  const url = new URL(request.url);
  const page = Number(url.searchParams.get("page") ?? "1");
  const from = (page - 1) * PAGE_SIZE;

  const { data, error } = await supabase
    .from("books")
    .select("*")
    .eq("owner_id", user.id)
    .order("created_at", { ascending: false })
    .range(from, from + PAGE_SIZE - 1);

  if (error) {
    return NextResponse.json({ error: error.message }, { status: 500 });
  }

  const books: Book[] = data.map((book) => ({
    ...book,
    coverUrl: coverUrlFor(book.isbn),
  }));

  return NextResponse.json({ books, page });
}

export async function POST(request: Request) {
  const user = await getCurrentUser();
  if (!user) {
    return NextResponse.json({ error: "Not logged in" }, { status: 401 });
  }

  const body = await request.json();
  if (!body.title || !body.author) {
    return NextResponse.json({ error: "Title and author are required" }, { status: 400 });
  }

  const { data, error } = await supabase
    .from("books")
    .insert({
      title: body.title,
      author: body.author,
      isbn: body.isbn ?? null,
      owner_id: user.id,
    })
    .select()
    .single();

  if (error) {
    return NextResponse.json({ error: error.message }, { status: 500 });
  }

  return NextResponse.json({ book: data }, { status: 201 });
}
