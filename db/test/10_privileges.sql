-- Droits par rôle, et immuabilité du journal et du registre des purges.
BEGIN;

SELECT t.check(NOT has_database_privilege('public', 'tracker', 'CONNECT'), 'PUBLIC ne peut pas se connecter');
SELECT t.check(has_table_privilege('tracker_app', 'core.records', 'INSERT'), 'tracker_app peut ajouter au journal');
SELECT t.check(NOT has_table_privilege('tracker_app', 'core.records', 'UPDATE'), 'tracker_app n''a pas UPDATE sur le journal');
SELECT t.check(NOT has_table_privilege('tracker_app', 'core.records', 'DELETE'), 'tracker_app n''a pas DELETE sur le journal');
SELECT t.check(NOT has_table_privilege('tracker_app', 'core.records', 'TRUNCATE'), 'tracker_app n''a pas TRUNCATE sur le journal');
SELECT t.check(NOT has_table_privilege('tracker_app', 'core.metrics', 'UPDATE'), 'tracker_app ne modifie pas les registres');
SELECT t.check(NOT has_table_privilege('tracker_app', 'core.purges', 'SELECT'), 'tracker_app ne voit pas le registre des purges');
SELECT t.check(has_column_privilege('tracker_admin', 'core.records', 'payload', 'UPDATE')
           AND has_column_privilege('tracker_admin', 'core.records', 'purge_id', 'UPDATE'),
               'tracker_admin peut rédiger payload et purge_id');
SELECT t.check(NOT has_column_privilege('tracker_admin', 'core.records', 'key', 'UPDATE')
           AND NOT has_column_privilege('tracker_admin', 'core.records', 'occurred_at', 'UPDATE'),
               'tracker_admin ne peut modifier aucune autre colonne du journal');
SELECT t.check(NOT has_table_privilege('tracker_admin', 'core.records', 'DELETE'), 'tracker_admin n''a pas DELETE sur le journal');

SELECT t.check(cache_size = 1, 'séquence du journal en CACHE 1')
FROM pg_sequences
WHERE format('%I.%I', schemaname, sequencename) = pg_get_serial_sequence('core.records', 'seq');

SELECT t.check(count(*) = 3, 'trois triggers actifs sur core.records')
FROM pg_trigger WHERE tgrelid = 'core.records'::regclass AND NOT tgisinternal AND tgenabled = 'O';
SELECT t.check(count(*) = 2, 'deux triggers actifs sur core.purges')
FROM pg_trigger WHERE tgrelid = 'core.purges'::regclass AND NOT tgisinternal AND tgenabled = 'O';

-- Immuabilité, rôle par rôle
SELECT t.make_source('immutability', '{coffee.cup}');
SELECT t.rec('immutability', 'coffee.cup', 'k1', '2026-10-17T08:00:00Z') AS seq1 \gset

SELECT t.expect_error('tracker_app', 'UPDATE core.records SET key = ''x''', '42501', 'tracker_app : UPDATE du journal');
SELECT t.expect_error('tracker_app', 'DELETE FROM core.records', '42501', 'tracker_app : DELETE du journal');
SELECT t.expect_error('tracker_app', 'TRUNCATE core.records', '42501', 'tracker_app : TRUNCATE du journal');

SELECT t.expect_error('tracker_owner', 'UPDATE core.records SET key = ''x''', 'LT001', 'propriétaire : UPDATE du journal');
SELECT t.expect_error('tracker_owner', 'DELETE FROM core.records', 'LT001', 'propriétaire : DELETE du journal');
SELECT t.expect_error('tracker_owner', 'TRUNCATE core.records CASCADE', 'LT001', 'propriétaire : TRUNCATE du journal');

SELECT t.expect_error('tracker_admin', 'UPDATE core.records SET key = ''x''', '42501', 'tracker_admin : modifier la clé');
SELECT t.expect_error('tracker_admin', format('UPDATE core.records SET payload = NULL WHERE seq = %s', :seq1),
                      'LT001', 'tracker_admin : vider la charge utile sans purge');

SELECT t.value_as('tracker_admin',
  $q$INSERT INTO core.purges (selector, reason, record_count) VALUES ('{"test": true}', 'test', 1) RETURNING id$q$) AS purge1 \gset
SELECT t.check(t.exec_as('tracker_admin',
  format('UPDATE core.records SET payload = NULL, purge_id = %s WHERE seq = %s', :purge1, :seq1)) = 1,
  'tracker_admin : une rédaction valide passe');
SELECT t.expect_error('tracker_admin',
  format('UPDATE core.records SET payload = ''{}'', purge_id = NULL WHERE seq = %s', :seq1),
  'LT001', 'tracker_admin : annuler une rédaction');
SELECT t.expect_error('tracker_owner',
  format('UPDATE core.records SET payload = NULL, purge_id = %s WHERE seq = %s', :purge1, :seq1),
  'LT001', 'seconde rédaction du même enregistrement');

SELECT t.expect_error('tracker_owner', 'UPDATE core.purges SET reason = ''x''', 'LT002', 'propriétaire : UPDATE du registre des purges');
SELECT t.expect_error('tracker_owner', 'DELETE FROM core.purges', 'LT002', 'propriétaire : DELETE du registre des purges');
SELECT t.expect_error('tracker_admin', 'DELETE FROM core.purges', '42501', 'tracker_admin : DELETE du registre des purges');

ROLLBACK;
