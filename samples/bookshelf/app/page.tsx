"use client";
import { useEffect, useState } from "react";
import { BookCard } from "@/components/BookCard";
import type { Book } from "@/types/book";

export default function HomePage() {
  const [books, setBooks] = useState<Book[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch("/api/books?page=1")
      .then((res) => res.json())
      .then((json) => setBooks(json.books ?? []))
      .finally(() => setLoading(false));
  }, []);

  if (loading) return <p className="p-8 text-neutral-500">Loading your shelf…</p>;

  return (
    <main className="grid grid-cols-2 gap-4 p-8 md:grid-cols-4">
      {books.map((book) => (
        <BookCard key={book.id} book={book} />
      ))}
    </main>
  );
}
