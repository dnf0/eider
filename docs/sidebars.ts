import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    {
      type: 'category',
      label: 'Getting Started',
      items: [
        'usage/overview',
        'usage/installation',
        'usage/quickstart-sql',
        'usage/quickstart-cli',
      ],
    },
    {
      type: 'category',
      label: 'Guides',
      items: ['usage/exporting'],
    },
    {
      type: 'category',
      label: 'SQL Reference',
      items: [
        'usage/sql_reference',
        'usage/sql_read_geo',
        'usage/sql_read_zarr_metadata',
        'usage/sql_plan_read_geo',
      ],
    },
    {
      type: 'category',
      label: 'CLI Reference',
      items: ['usage/cli_tui'],
    },
    {
      type: 'category',
      label: 'Concepts & Engineering',
      items: [
        'engineering/architecture',
        'engineering/spatial_pruning',
        'engineering/cog_virtualization',
        'engineering/benchmarks',
      ],
    },
  ],
};

export default sidebars;
