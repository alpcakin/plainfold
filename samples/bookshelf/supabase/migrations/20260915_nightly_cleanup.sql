select cron.schedule(
  'nightly-cleanup',
  '0 3 * * *',
  $$
  select net.http_post(
    url := 'https://bookshelf.supabase.co/functions/v1/cleanup-deleted',
    headers := jsonb_build_object('Authorization', 'Bearer ' || current_setting('app.service_key'))
  );
  $$
);
