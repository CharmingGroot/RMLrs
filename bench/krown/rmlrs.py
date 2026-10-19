#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 AND MIT
# Based on bench_executor/morphkgc.py from kg-construct/KROWN,
# Copyright (c) 2022-2024 Dylan Van Assche, MIT License (see NOTICE).
"""
RMLrs resource for KROWN's execution framework (kg-construct/KROWN).

Copy this file into `execution-framework/bench_executor/`; scenarios then
use `"resource": "RMLrs"` with the same `execute_mapping` parameters as the
other engines.

KROWN's data generator writes mappings in the legacy R2RML / old RML
vocabulary, which the RMLrs engine does not read (that is milestone M6).
Before starting the container, the mapping is rewritten to the current RML
vocabulary next to the original (`<name>.rmlrs.ttl`). The rewrite touches
only vocabulary IRIs and takes milliseconds; it runs outside the measured
container, so it is not part of the engine's time.

For database scenarios, KROWN passes the connection to the engine
(`rdb_*` parameters) and the mapping only names the table
(`rr:logicalTable [ rr:tableName "data" ]`). The rewrite turns each logical
table into an RML logical source over a `d2rq:Database` built from those
parameters, which is how RML-IO-Registry describes database sources.

**Repository**: https://github.com/CharmingGroot/RMLrs
"""

import os
from typing import Optional
from timeout_decorator import timeout, TimeoutError  # type: ignore
from rdflib import BNode, Graph, Literal, Namespace, URIRef  # type: ignore
from rdflib.namespace import RDF  # type: ignore
from bench_executor.container import Container
from bench_executor.logger import Logger

IMAGE = 'rmlrs-krown/rmlrs'
VERSION = 'dev'
TIMEOUT = 6 * 3600  # 6 hours

R2RML = 'http://www.w3.org/ns/r2rml#'
OLD_RML = 'http://semweb.mmlab.be/ns/rml#'
QL = 'http://semweb.mmlab.be/ns/ql#'
RML = 'http://w3id.org/rml/'
D2RQ = Namespace('http://www.wiwiss.fu-berlin.de/suhl/bizer/D2RQ/0.1#')
RENAME = {'ReferenceObjectMap': 'RefObjectMap', 'column': 'reference'}
JDBC = {'PostgreSQL': ('postgresql', 'org.postgresql.Driver'),
        'MySQL': ('mysql', 'com.mysql.cj.jdbc.Driver')}


def _to_current_rml(term):
    if isinstance(term, URIRef):
        s = str(term)
        for ns in (R2RML, OLD_RML, QL):
            if s.startswith(ns):
                local = s[len(ns):]
                return URIRef(RML + RENAME.get(local, local))
    return term


class RMLrs(Container):
    """RMLrs container for executing RML mappings."""

    def __init__(self, data_path: str, config_path: str, directory: str,
                 verbose: bool):
        """Creates an instance of the RMLrs class.

        Parameters
        ----------
        data_path : str
            Path to the data directory of the case.
        config_path : str
            Path to the config directory of the case.
        directory : str
            Path to the directory to store logs.
        verbose : bool
            Enable verbose logs.
        """
        self._data_path = os.path.abspath(data_path)
        self._config_path = os.path.abspath(config_path)
        self._logger = Logger(__name__, directory, verbose)
        os.umask(0)
        os.makedirs(os.path.join(self._data_path, 'rmlrs'), exist_ok=True)
        super().__init__(f'{IMAGE}:{VERSION}', 'RMLrs', self._logger,
                         volumes=[f'{self._data_path}/rmlrs:/data',
                                  f'{self._data_path}/shared:/data/shared'])

    @property
    def root_mount_directory(self) -> str:
        """Subdirectory in the root directory of the case for RMLrs."""
        return __name__.lower()

    @timeout(TIMEOUT)
    def _execute_with_timeout(self, arguments: list) -> bool:
        """Run RMLrs in its container and wait for it to exit."""
        cmd = f'rmlrs run {" ".join(arguments)}'
        self._logger.debug(f'Executing RMLrs with arguments {arguments}')
        return self.run_and_wait_for_exit(cmd)

    def execute(self, arguments: list) -> bool:
        """Execute RMLrs with given arguments.

        Parameters
        ----------
        arguments : list
            Arguments to supply to the `rmlrs run` command.

        Returns
        -------
        success : bool
            Whether the execution succeeded or not.
        """
        try:
            return self._execute_with_timeout(arguments)
        except TimeoutError:
            self._logger.warning(f'Timeout ({TIMEOUT}s) reached for RMLrs')
        return False

    def execute_mapping(self,
                        mapping_file: str,
                        output_file: str,
                        serialization: str,
                        rdb_username: Optional[str] = None,
                        rdb_password: Optional[str] = None,
                        rdb_host: Optional[str] = None,
                        rdb_port: Optional[int] = None,
                        rdb_name: Optional[str] = None,
                        rdb_type: Optional[str] = None) -> bool:
        """Execute a mapping file with RMLrs.

        RMLrs writes N-Quads. Without named graphs, N-Quads lines are also
        N-Triples lines, so both `nquads` and `ntriples` are accepted.
        For database scenarios, the rdb_* parameters describe the database
        that the mapping's logical tables are read from.

        Parameters
        ----------
        mapping_file : str
            Path to the mapping file to execute, relative to the shared
            data directory.
        output_file : str
            Name of the output file to store the triples in.
        serialization : str
            Serialization format to use: `nquads` or `ntriples`.

        Returns
        -------
        success : bool
            Whether the execution was successfull or not.
        """
        if serialization not in ('nquads', 'ntriples'):
            raise NotImplementedError('Unsupported serialization: '
                                      f'"{serialization}"')
        shared = os.path.join(self._data_path, 'shared')
        legacy = Graph().parse(os.path.join(shared, mapping_file),
                               format='turtle')
        current = Graph()
        current.bind('rml', RML)
        current.bind('d2rq', D2RQ)
        tables = list(legacy.subject_objects(URIRef(R2RML + 'logicalTable')))
        if tables:
            if rdb_type not in JDBC or rdb_host is None:
                self._logger.error('RMLrs resource: a logical table needs '
                                   'database parameters (PostgreSQL or MySQL)')
                return False
            kind, driver = JDBC[rdb_type]
            db = BNode()
            current.add((db, RDF.type, D2RQ.Database))
            current.add((db, D2RQ.jdbcDSN, Literal(
                f'jdbc:{kind}://{rdb_host}:{rdb_port}/{rdb_name}')))
            current.add((db, D2RQ.jdbcDriver, Literal(driver)))
            current.add((db, D2RQ.username, Literal(rdb_username)))
            current.add((db, D2RQ.password, Literal(rdb_password)))
        dropped = set()
        for tm, table in tables:
            dropped.add(table)
            source = BNode()
            current.add((tm, URIRef(RML + 'logicalSource'), source))
            current.add((source, URIRef(RML + 'source'), db))
            name = legacy.value(table, URIRef(R2RML + 'tableName'))
            query = legacy.value(table, URIRef(R2RML + 'sqlQuery'))
            formulation = 'SQL2008Table' if name is not None else 'SQL2008Query'
            current.add((source, URIRef(RML + 'referenceFormulation'),
                         URIRef(RML + formulation)))
            current.add((source, URIRef(RML + 'iterator'),
                         Literal(str(name if name is not None else query))))
        for s, p, o in legacy:
            if s in dropped or p == URIRef(R2RML + 'logicalTable'):
                continue
            current.add((_to_current_rml(s), _to_current_rml(p),
                         _to_current_rml(o)))
        name = os.path.splitext(os.path.basename(mapping_file))[0]
        rewritten = f'{name}.rmlrs.ttl'
        current.serialize(os.path.join(shared, rewritten), format='turtle')

        return self.execute([f'/data/shared/{rewritten}',
                             '-o', f'/data/shared/{output_file}'])
