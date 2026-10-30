import Link from "next/link";
import type { Book } from "@/types/book";

export function BookCard({ book }: { book: Book }) {
  return (
    <Link href={`/books/${book.id}`} className="rounded-xl border p-3 hover:shadow">
      {book.coverUrl ? (
        <img src={book.coverUrl} alt={book.title} className="aspect-[2/3] rounded-md" />
      ) : (
        <div className="aspect-[2/3] rounded-md bg-neutral-200" />
      )}
      <h3 className="mt-2 font-semibold">{book.title}</h3>
      <p className="text-sm text-neutral-500">{book.author}</p>
    </Link>
  );
}
