# dbt naming

| Layer | Pattern | Example |
| --- | --- | --- |
| Staging | `stg_<source>__<entity>` | `stg_billing__invoices` |
| Intermediate | `int_<entity>_<verb>` | `int_invoices_joined` |
| Mart | `<entity>` or `fct_`/`dim_` prefix | `fct_payments` |
