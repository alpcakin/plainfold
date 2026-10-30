"use client";
import { useState } from "react";
import { useRouter } from "next/navigation";

export default function NewBookPage() {
  const router = useRouter();
  const [error, setError] = useState("");

  async function save(form: FormData) {
    const res = await fetch("/api/books", {
      method: "POST",
      body: JSON.stringify({
        title: form.get("title"),
        author: form.get("author"),
        isbn: form.get("isbn") || null,
      }),
    });
    if (!res.ok) {
      setError((await res.json()).error);
      return;
    }
    router.push("/");
  }

  return (
    <form action={save} className="grid gap-3 p-8">
      <input name="title" placeholder="Title" />
      <input name="author" placeholder="Author" />
      <input name="isbn" placeholder="ISBN (optional)" />
      {error && <p className="text-red-600">{error}</p>}
      <button type="submit">Save</button>
    </form>
  );
}
