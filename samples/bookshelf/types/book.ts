export type Book = {
  id: string;
  title: string;
  author: string;
  isbn: string | null;
  owner_id: string;
  created_at: string;
  deleted_at: string | null;
  coverUrl?: string | null;
};
