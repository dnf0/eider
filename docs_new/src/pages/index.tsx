import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <Heading as="h1" className="hero__title" style={{color: 'white'}}>
          {siteConfig.title}
        </Heading>
        <p className="hero__subtitle" style={{color: 'white'}}>{siteConfig.tagline}</p>
        <div className={styles.buttons}>
          <Link
            className="button button--secondary button--lg"
            to="/docs/usage/installation">
            Quick Start
          </Link>
        </div>
      </div>
    </header>
  );
}

export default function Home(): JSX.Element {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title={`Home`}
      description="Zero-Copy Cloud Data in DuckDB">
      <HomepageHeader />
      <main>
        <div className="container margin-vert--xl text--center">
          <div className="row">
            <div className="col col--10 col--offset-1">
              <h2>A Native DuckDB Extension for Zarr & GeoZarr</h2>
              <p>
                Eider connects DuckDB's vectorized execution engine directly to multi-dimensional arrays in cloud storage via OpenDAL and Zarrs. It bridges the gap between data-science Python pipelines and fast analytical SQL.
              </p>
              <div className="glass-panel margin-top--lg margin-bottom--xl">
                <img src="/eider/img/demo.gif" alt="Eider STAC Discovery TUI Demo" style={{borderRadius: '8px'}} />
              </div>
            </div>
          </div>
          
          <div className="row text--left margin-top--lg">
            <div className="col col--4">
              <h3>Fast & Vectorized</h3>
              <p>Chunks are decoded natively inside DuckDB's engine, eliminating Python IPC overhead. Spatial bounding box pruning drops network requests before they start.</p>
            </div>
            <div className="col col--4">
              <h3>Cloud Native</h3>
              <p>Supports <code>s3://</code>, <code>gcs://</code>, and <code>http://</code> streams natively via OpenDAL. Automatically fetches partial chunks from remote systems.</p>
            </div>
            <div className="col col--4">
              <h3>COG Virtualization</h3>
              <p>Transparently read Cloud Optimized GeoTIFFs (COGs) as if they were Zarr stores, intercepting headers and making native HTTP byte-range fetches.</p>
            </div>
          </div>
        </div>
      </main>
    </Layout>
  );
}
