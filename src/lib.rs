use async_trait::async_trait;
use diesel::{
    connection::SimpleConnection,
    dsl::Limit,
    query_dsl::{
        methods::{ExecuteDsl, LimitDsl, LoadQuery},
        RunQueryDsl,
    },
    r2d2::{ConnectionManager, Pool, R2D2Connection},
    result::QueryResult,
    Connection,
};
use std::{error::Error as StdError, fmt};
use tokio::task;

pub type AsyncResult<R> = Result<R, AsyncError>;

#[derive(Debug)]
pub enum AsyncError {
    // Failed to checkout a connection
    Checkout(r2d2::Error),

    // The query failed in some way
    Error(diesel::result::Error),
}

pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>, AsyncError>;
}

impl<T> OptionalExtension<T> for AsyncResult<T> {
    fn optional(self) -> Result<Option<T>, AsyncError> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(AsyncError::Error(diesel::result::Error::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl fmt::Display for AsyncError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            AsyncError::Checkout(ref err) => err.fmt(f),
            AsyncError::Error(ref err) => err.fmt(f),
        }
    }
}

impl StdError for AsyncError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match *self {
            AsyncError::Checkout(ref err) => Some(err),
            AsyncError::Error(ref err) => Some(err),
        }
    }
}

#[async_trait]
pub trait AsyncSimpleConnection<Conn>
where
    Conn: 'static + SimpleConnection + R2D2Connection,
{
    async fn batch_execute_async(&self, query: &str) -> AsyncResult<()>;
}

#[async_trait]
impl<Conn> AsyncSimpleConnection<Conn> for Pool<ConnectionManager<Conn>>
where
    Conn: 'static + Connection + R2D2Connection,
{
    #[inline]
    async fn batch_execute_async(&self, query: &str) -> AsyncResult<()> {
        let self_ = self.clone();
        let query = query.to_string();
        task::block_in_place(move || {
            let mut conn = self_.get().map_err(AsyncError::Checkout)?;
            conn.batch_execute(&query).map_err(AsyncError::Error)
        })
    }
}

#[async_trait]
pub trait AsyncConnection<Conn>: AsyncSimpleConnection<Conn>
where
    Conn: 'static + Connection + R2D2Connection,
{
    async fn run<R, Func>(&self, f: Func) -> AsyncResult<R>
    where
        R: Send,
        Func: FnOnce(&mut Conn) -> QueryResult<R> + Send;

    async fn transaction<R, Func>(&self, f: Func) -> AsyncResult<R>
    where
        R: Send,
        Func: FnOnce(&mut Conn) -> QueryResult<R> + Send;
}

#[async_trait]
impl<Conn> AsyncConnection<Conn> for Pool<ConnectionManager<Conn>>
where
    Conn: 'static + Connection + R2D2Connection,
{
    #[inline]
    async fn run<R, Func>(&self, f: Func) -> AsyncResult<R>
    where
        R: Send,
        Func: FnOnce(&mut Conn) -> QueryResult<R> + Send,
    {
        let self_ = self.clone();
        task::block_in_place(move || {
            let mut conn = self_.get().map_err(AsyncError::Checkout)?;
            f(&mut *conn).map_err(AsyncError::Error)
        })
    }

    #[inline]
    async fn transaction<R, Func>(&self, f: Func) -> AsyncResult<R>
    where
        R: Send,
        Func: FnOnce(&mut Conn) -> QueryResult<R> + Send,
    {
        let self_ = self.clone();
        task::block_in_place(move || {
            let mut conn = self_.get().map_err(AsyncError::Checkout)?;
            conn.transaction(|conn| f(&mut *conn)).map_err(AsyncError::Error)
        })
    }
}

#[async_trait]
pub trait AsyncRunQueryDsl<'query, Conn, AsyncConn>
where
    Conn: 'static + Connection + R2D2Connection,
{
    async fn execute_async(self, asc: &AsyncConn) -> AsyncResult<usize>
    where
        Self: ExecuteDsl<Conn>;

    async fn load_async<U>(self, asc: &AsyncConn) -> AsyncResult<Vec<U>>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>;

    async fn get_result_async<U>(self, asc: &AsyncConn) -> AsyncResult<U>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>;

    async fn get_results_async<U>(self, asc: &AsyncConn) -> AsyncResult<Vec<U>>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>;

    async fn first_async<U>(self, asc: &AsyncConn) -> AsyncResult<U>
    where
        U: Send,
        Self: LimitDsl,
        Limit<Self>: LoadQuery<'query, Conn, U>;
}

#[async_trait]
impl<'query, T, Conn> AsyncRunQueryDsl<'query, Conn, Pool<ConnectionManager<Conn>>> for T
where
    T: Send + RunQueryDsl<Conn>,
    Conn: 'static + Connection + R2D2Connection,
{
    async fn execute_async(self, asc: &Pool<ConnectionManager<Conn>>) -> AsyncResult<usize>
    where
        Self: ExecuteDsl<Conn>,
    {
        asc.run(|conn| self.execute(conn)).await
    }

    async fn load_async<U>(self, asc: &Pool<ConnectionManager<Conn>>) -> AsyncResult<Vec<U>>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>,
    {
        asc.run(|mut conn| self.load(&mut conn)).await
    }

    async fn get_result_async<U>(self, asc: &Pool<ConnectionManager<Conn>>) -> AsyncResult<U>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>,
    {
        asc.run(|mut conn| self.get_result(&mut conn)).await
    }

    async fn get_results_async<U>(self, asc: &Pool<ConnectionManager<Conn>>) -> AsyncResult<Vec<U>>
    where
        U: Send,
        Self: LoadQuery<'query, Conn, U>,
    {
        asc.run(|mut conn| self.get_results(&mut conn)).await
    }

    async fn first_async<U>(self, asc: &Pool<ConnectionManager<Conn>>) -> AsyncResult<U>
    where
        U: Send,
        Self: LimitDsl,
        Limit<Self>: LoadQuery<'query, Conn, U>,
    {
        asc.run(|mut conn| self.first(&mut conn)).await
    }
}
